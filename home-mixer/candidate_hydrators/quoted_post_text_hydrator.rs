use crate::clients::tweet_entity_service_client::TESClient;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::hydrator::Hydrator;

pub struct QuotedPostTextHydrator {
    pub tes_client: Arc<dyn TESClient + Send + Sync>,
}

impl QuotedPostTextHydrator {
    pub fn new(tes_client: Arc<dyn TESClient + Send + Sync>) -> Self {
        Self { tes_client }
    }
}

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for QuotedPostTextHydrator {
    async fn hydrate(
        &self,
        _query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let fetch_ids: Vec<u64> = candidates
            .iter()
            .flat_map(|c| c.quoted_tweet_id.into_iter().chain(c.ancestors.iter().copied()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let core = if fetch_ids.is_empty() {
            HashMap::new()
        } else {
            self.tes_client.get_tweet_core_datas(fetch_ids).await
        };

        candidates
            .iter()
            .map(|candidate| {
                Ok(PostCandidate {
                    quoted_tweet_text: candidate
                        .quoted_tweet_id
                        .and_then(|id| text_from_core(&core, id)),
                    ancestor_texts: candidate
                        .ancestors
                        .iter()
                        .copied()
                        .filter_map(|id| text_from_core(&core, id).map(|text| (id, text)))
                        .collect(),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.quoted_tweet_text = hydrated.quoted_tweet_text;
        candidate.ancestor_texts = hydrated.ancestor_texts;
    }
}

fn text_from_core<E>(
    core: &HashMap<u64, Result<Option<xai_core_entities::entities::PureCoreData>, E>>,
    id: u64,
) -> Option<String> {
    match core.get(&id) {
        Some(Ok(Some(data))) if !data.text.is_empty() => Some(data.text.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::tweet_entity_service_client::MockTESClient;
    use xai_core_entities::entities::PureCoreData;

    #[tokio::test]
    async fn fills_quoted_text_and_leaves_non_quotes_empty() {
        let mut core_data = HashMap::new();
        core_data.insert(
            99,
            Some(PureCoreData {
                text: "quoted text".to_string(),
                ..Default::default()
            }),
        );
        let client = Arc::new(MockTESClient {
            core_data,
            ..Default::default()
        });
        let hydrator = QuotedPostTextHydrator::new(client as Arc<dyn TESClient + Send + Sync>);

        let mut with_quote = PostCandidate {
            tweet_id: 1,
            quoted_tweet_id: Some(99),
            ..Default::default()
        };
        let mut without_quote = PostCandidate {
            tweet_id: 2,
            ..Default::default()
        };

        let hydrated = hydrator
            .hydrate(
                &ScoredPostsQuery::default(),
                &[with_quote.clone(), without_quote.clone()],
            )
            .await;
        hydrator.update(&mut with_quote, hydrated[0].clone().unwrap());
        hydrator.update(&mut without_quote, hydrated[1].clone().unwrap());

        assert_eq!(with_quote.quoted_tweet_text.as_deref(), Some("quoted text"));
        assert_eq!(without_quote.quoted_tweet_text, None);
        assert!(with_quote.ancestor_texts.is_empty());
    }

    #[tokio::test]
    async fn fills_ancestor_texts_without_changing_ancestor_ids() {
        let mut core_data = HashMap::new();
        core_data.insert(
            20,
            Some(PureCoreData {
                text: "parent spam".to_string(),
                ..Default::default()
            }),
        );
        core_data.insert(
            10,
            Some(PureCoreData {
                text: "root text".to_string(),
                ..Default::default()
            }),
        );
        let client = Arc::new(MockTESClient {
            core_data,
            ..Default::default()
        });
        let hydrator = QuotedPostTextHydrator::new(client as Arc<dyn TESClient + Send + Sync>);

        let mut reply = PostCandidate {
            tweet_id: 30,
            ancestors: vec![20, 10],
            ..Default::default()
        };

        let hydrated = hydrator
            .hydrate(&ScoredPostsQuery::default(), &[reply.clone()])
            .await;
        hydrator.update(&mut reply, hydrated[0].clone().unwrap());

        assert_eq!(reply.ancestors, vec![20, 10]);
        assert_eq!(reply.ancestor_texts.get(&20).map(String::as_str), Some("parent spam"));
        assert_eq!(reply.ancestor_texts.get(&10).map(String::as_str), Some("root text"));
        assert_eq!(reply.quoted_tweet_text, None);
    }
}
