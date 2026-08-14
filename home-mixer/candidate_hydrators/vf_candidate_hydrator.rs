use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::params::EnableXaiVfClient;
use anyhow::Result;
use futures::future::join;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::hydrator::Hydrator;
use xai_twittercontext_proto::GetTwitterContextViewer;
use xai_twittercontext_proto::TwitterContextViewer;
use xai_visibility_filtering::models::{Action, FilteredReason};
use xai_visibility_filtering::vf_client::SafetyLevel;
use xai_visibility_filtering::vf_client::SafetyLevel::{TimelineHome, TimelineHomeRecommendations};
use xai_visibility_filtering::vf_client::VfClient;

pub struct VFCandidateHydrator {
    pub strato_vf_client: Arc<dyn VfClient + Send + Sync>,
    pub xai_vf_client: Arc<dyn VfClient + Send + Sync>,
}

impl VFCandidateHydrator {
    pub async fn new(
        strato_vf_client: Arc<dyn VfClient + Send + Sync>,
        xai_vf_client: Arc<dyn VfClient + Send + Sync>,
    ) -> Self {
        Self {
            strato_vf_client,
            xai_vf_client,
        }
    }

    async fn fetch_vf_results(
        client: &Arc<dyn VfClient + Send + Sync>,
        tweet_ids: Vec<u64>,
        safety_level: SafetyLevel,
        for_user_id: u64,
        context: Option<TwitterContextViewer>,
    ) -> HashMap<u64, Result<Option<FilteredReason>>> {
        if tweet_ids.is_empty() {
            return HashMap::new();
        }

        client
            .get_result(tweet_ids, safety_level, for_user_id, context)
            .await
    }
}

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for VFCandidateHydrator {
    async fn hydrate(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let context = query.get_viewer();
        let user_id = query.user_id;
        // Fully migrated to Rust VF. Old VF available in the event of production issues.
        let client = if query.params.get(EnableXaiVfClient) {
            &self.xai_vf_client
        } else {
            &self.strato_vf_client
        };

        let mut in_network_ids: Vec<u64> = Vec::new();
        let mut oon_ids: Vec<u64> = Vec::new();

        for candidate in candidates.iter() {
            if candidate.in_network.unwrap_or(false) {
                in_network_ids.push(candidate.tweet_id);
            } else {
                oon_ids.push(candidate.tweet_id);
            }
            for &ancestor_id in &candidate.ancestors {
                oon_ids.push(ancestor_id);
            }
            if let Some(quoted_id) = candidate.quoted_tweet_id {
                oon_ids.push(quoted_id);
            }
            if let Some(retweeted_id) = candidate.retweeted_tweet_id {
                in_network_ids.push(retweeted_id);
            }
        }

        oon_ids.sort_unstable();
        oon_ids.dedup();

        let in_network_future = Self::fetch_vf_results(
            client,
            in_network_ids,
            TimelineHome,
            user_id,
            context.clone(),
        );

        let oon_future = Self::fetch_vf_results(
            client,
            oon_ids,
            TimelineHomeRecommendations,
            user_id,
            context,
        );

        let (in_network_result, oon_result) = join(in_network_future, oon_future).await;

        let mut hydrated_candidates = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            // Read the primary tweet from the map that matches its lane. Merging
            // Recommendations over TimelineHome overwrites an in-network parent
            // that is also another selected candidate's ancestor or quote.
            let primary_result = if candidate.in_network.unwrap_or(false) {
                in_network_result.get(&candidate.tweet_id)
            } else {
                oon_result.get(&candidate.tweet_id)
            };
            let visibility_reason = match primary_result {
                Some(Ok(Some(reason))) => Some(reason.clone()),
                _ => None,
            };

            let drop_ancillary = should_drop_ancillary(candidate, &in_network_result, &oon_result);

            let hydrated = match primary_result {
                Some(Err(err)) => Err(err.to_string()),
                _ => Ok(PostCandidate {
                    visibility_reason,
                    drop_ancillary_posts: Some(drop_ancillary),
                    ..Default::default()
                }),
            };
            hydrated_candidates.push(hydrated);
        }
        hydrated_candidates
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.visibility_reason = hydrated.visibility_reason;
        candidate.drop_ancillary_posts = hydrated.drop_ancillary_posts;
    }
}

fn should_drop_ancillary(
    candidate: &PostCandidate,
    in_network_results: &HashMap<u64, Result<Option<FilteredReason>>>,
    oon_results: &HashMap<u64, Result<Option<FilteredReason>>>,
) -> bool {
    for &ancestor_id in &candidate.ancestors {
        if candidate.tombstone_ancestor_ids.contains(&ancestor_id) {
            continue;
        }
        if let Some(Ok(Some(reason))) = oon_results.get(&ancestor_id)
            && should_drop_reason(reason)
        {
            return true;
        }
    }

    if let Some(quoted_id) = candidate.quoted_tweet_id
        && let Some(Ok(Some(reason))) = oon_results.get(&quoted_id)
        && should_drop_reason(reason)
    {
        return true;
    }

    if let Some(retweeted_id) = candidate.retweeted_tweet_id
        && let Some(Ok(Some(reason))) = in_network_results.get(&retweeted_id)
        && should_drop_reason(reason)
    {
        return true;
    }

    false
}

fn should_drop_reason(reason: &FilteredReason) -> bool {
    match reason {
        FilteredReason::SafetyResult(safety_result) => {
            matches!(safety_result.action, Action::Drop(_))
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_visibility_filtering::models::SafetyResult;

    struct LevelAwareVfClient {
        timeline_home: HashMap<u64, FilteredReason>,
        recommendations: HashMap<u64, FilteredReason>,
    }

    #[async_trait]
    impl VfClient for LevelAwareVfClient {
        async fn get_result(
            &self,
            tweet_ids: Vec<u64>,
            safety_level: SafetyLevel,
            _for_user_id: u64,
            _context: Option<TwitterContextViewer>,
        ) -> HashMap<u64, Result<Option<FilteredReason>>> {
            let level_map = match safety_level {
                TimelineHome => &self.timeline_home,
                TimelineHomeRecommendations => &self.recommendations,
                _ => return HashMap::new(),
            };
            tweet_ids
                .into_iter()
                .filter_map(|id| {
                    level_map
                        .get(&id)
                        .cloned()
                        .map(|reason| (id, Ok(Some(reason))))
                })
                .collect()
        }
    }

    fn safety_reason(action: Action) -> FilteredReason {
        FilteredReason::SafetyResult(SafetyResult {
            action,
            ..Default::default()
        })
    }

    fn hydrator(client: LevelAwareVfClient) -> VFCandidateHydrator {
        let client: Arc<dyn VfClient + Send + Sync> = Arc::new(client);
        VFCandidateHydrator {
            strato_vf_client: Arc::clone(&client),
            xai_vf_client: client,
        }
    }

    async fn hydrate(
        hydrator: &VFCandidateHydrator,
        candidates: &[PostCandidate],
    ) -> Vec<PostCandidate> {
        hydrator
            .hydrate(&ScoredPostsQuery::default(), candidates)
            .await
            .into_iter()
            .map(|result| result.expect("vf hydrate should succeed"))
            .collect()
    }

    #[tokio::test]
    async fn in_network_parent_keeps_timeline_home_when_also_reply_ancestor() {
        let parent_id = 10;
        let reply_id = 11;
        let oon_id = 20;
        let client = LevelAwareVfClient {
            timeline_home: HashMap::from([
                (parent_id, safety_reason(Action::Interstitial)),
                (reply_id, safety_reason(Action::Allow)),
            ]),
            recommendations: HashMap::from([
                (parent_id, safety_reason(Action::Drop(Default::default()))),
                (oon_id, safety_reason(Action::Drop(Default::default()))),
            ]),
        };
        let hydrator = hydrator(client);
        let candidates = vec![
            PostCandidate {
                tweet_id: parent_id,
                in_network: Some(true),
                ..Default::default()
            },
            PostCandidate {
                tweet_id: reply_id,
                in_network: Some(true),
                ancestors: vec![parent_id],
                ..Default::default()
            },
            PostCandidate {
                tweet_id: oon_id,
                in_network: Some(false),
                ..Default::default()
            },
        ];

        let hydrated = hydrate(&hydrator, &candidates).await;

        assert_eq!(
            hydrated[0].visibility_reason,
            Some(safety_reason(Action::Interstitial)),
            "in-network parent that is also a sibling reply ancestor must keep TimelineHome"
        );
        assert_eq!(
            hydrated[1].visibility_reason,
            Some(safety_reason(Action::Allow))
        );
        assert_eq!(
            hydrated[1].drop_ancillary_posts,
            Some(true),
            "ancestor/quote ancillary checks still use TimelineHomeRecommendations"
        );
        assert_eq!(
            hydrated[2].visibility_reason,
            Some(safety_reason(Action::Drop(Default::default()))),
            "true OON primary must use TimelineHomeRecommendations"
        );
    }

    #[tokio::test]
    async fn oon_primary_uses_recommendations_even_when_timeline_home_would_allow() {
        let oon_id = 20;
        let client = LevelAwareVfClient {
            timeline_home: HashMap::from([(oon_id, safety_reason(Action::Allow))]),
            recommendations: HashMap::from([(
                oon_id,
                safety_reason(Action::Drop(Default::default())),
            )]),
        };
        let hydrator = hydrator(client);
        let candidates = vec![PostCandidate {
            tweet_id: oon_id,
            in_network: Some(false),
            ..Default::default()
        }];

        let hydrated = hydrate(&hydrator, &candidates).await;

        assert_eq!(
            hydrated[0].visibility_reason,
            Some(safety_reason(Action::Drop(Default::default())))
        );
    }
}
