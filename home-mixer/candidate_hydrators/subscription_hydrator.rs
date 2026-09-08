use crate::clients::tweet_entity_service_client::TESClient;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::utils::{default_quick_cache, QuickCache};
use xai_candidate_pipeline::hydrator::{CacheStore, CachedHydrator};

pub struct SubscriptionHydrator {
    pub tes_client: Arc<dyn TESClient + Send + Sync>,
    pub cache: QuickCache<u64, Option<u64>>,
}

impl SubscriptionHydrator {
    pub async fn new(tes_client: Arc<dyn TESClient + Send + Sync>) -> Self {
        let cache = default_quick_cache();
        Self { tes_client, cache }
    }
}

#[async_trait]
impl CachedHydrator<ScoredPostsQuery, PostCandidate> for SubscriptionHydrator {
    type CacheKey = u64;
    type CacheValue = Option<u64>;

    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        !query.has_cached_posts
    }

    fn cache_store(&self) -> &dyn CacheStore<Self::CacheKey, Self::CacheValue> {
        &self.cache
    }
    fn cache_key(&self, candidate: &PostCandidate) -> Self::CacheKey {
        candidate.tweet_id
    }

    fn cache_value(&self, hydrated: &PostCandidate) -> Self::CacheValue {
        hydrated.subscription_author_id
    }

    fn hydrate_from_cache(&self, value: Self::CacheValue) -> PostCandidate {
        PostCandidate {
            subscription_author_id: value,
            ..Default::default()
        }
    }

    async fn hydrate_from_client(
        &self,
        _query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let client = &self.tes_client;

        let tweet_ids = subscription_fetch_ids(candidates);

        let post_features = client.get_subscription_author_ids(tweet_ids).await;

        candidates
            .iter()
            .map(|candidate| {
                resolve_subscription_author_id(&post_features, candidate).map(|value| {
                    PostCandidate {
                        subscription_author_id: value,
                        ..Default::default()
                    }
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.subscription_author_id = hydrated.subscription_author_id;
    }
}

fn subscription_fetch_ids(candidates: &[PostCandidate]) -> Vec<u64> {
    let mut ids = Vec::with_capacity(candidates.len() * 2);
    for candidate in candidates {
        ids.push(candidate.tweet_id);
        if let Some(quoted_id) = candidate.quoted_tweet_id {
            if quoted_id != candidate.tweet_id {
                ids.push(quoted_id);
            }
        }
    }
    ids
}

fn resolve_subscription_author_id<E: ToString>(
    tes: &HashMap<u64, Result<Option<u64>, E>>,
    candidate: &PostCandidate,
) -> Result<Option<u64>, String> {
    let own = match tes.get(&candidate.tweet_id) {
        Some(Ok(value)) => *value,
        None => {
            return Err(format!(
                "Missing subscription author id for tweet_id={}",
                candidate.tweet_id
            ));
        }
        Some(Err(err)) => return Err(err.to_string()),
    };

    if own.is_some() {
        return Ok(own);
    }

    if let Some(quoted_id) = candidate.quoted_tweet_id {
        if let Some(Ok(Some(id))) = tes.get(&quoted_id) {
            return Ok(Some(*id));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_exclusive_still_uses_candidate_id() {
        let candidates = vec![PostCandidate {
            tweet_id: 20,
            ..Default::default()
        }];
        assert_eq!(subscription_fetch_ids(&candidates), vec![20]);
    }

    #[test]
    fn quote_of_exclusive_fetches_quoted_id() {
        let candidates = vec![PostCandidate {
            tweet_id: 10,
            quoted_tweet_id: Some(20),
            ..Default::default()
        }];
        assert_eq!(subscription_fetch_ids(&candidates), vec![10, 20]);
    }

    #[test]
    fn retweet_without_quote_does_not_use_original_id() {
        let candidates = vec![PostCandidate {
            tweet_id: 10,
            retweeted_tweet_id: Some(20),
            ..Default::default()
        }];
        assert_eq!(subscription_fetch_ids(&candidates), vec![10]);
    }

    #[test]
    fn tes_keyed_only_by_wrapper_does_not_mark_quote_exclusive() {
        let candidate = PostCandidate {
            tweet_id: 10,
            quoted_tweet_id: Some(20),
            ..Default::default()
        };
        let mut tes = HashMap::new();
        tes.insert(10, Ok(None));
        assert_eq!(
            resolve_subscription_author_id(&tes, &candidate).unwrap(),
            None
        );
    }

    #[test]
    fn quote_of_exclusive_uses_quoted_author() {
        let candidate = PostCandidate {
            tweet_id: 10,
            quoted_tweet_id: Some(20),
            ..Default::default()
        };
        let mut tes = HashMap::new();
        tes.insert(10, Ok(None));
        tes.insert(20, Ok(Some(99)));
        assert_eq!(
            resolve_subscription_author_id(&tes, &candidate).unwrap(),
            Some(99)
        );
    }

    #[test]
    fn exclusive_quote_of_public_keeps_wrapper_author() {
        let candidate = PostCandidate {
            tweet_id: 10,
            quoted_tweet_id: Some(20),
            ..Default::default()
        };
        let mut tes = HashMap::new();
        tes.insert(10, Ok(Some(7)));
        tes.insert(20, Ok(None));
        assert_eq!(
            resolve_subscription_author_id(&tes, &candidate).unwrap(),
            Some(7)
        );
    }

    #[test]
    fn native_not_exclusive_stays_none() {
        let candidate = PostCandidate {
            tweet_id: 20,
            ..Default::default()
        };
        let mut tes = HashMap::new();
        tes.insert(20, Ok(None));
        assert_eq!(
            resolve_subscription_author_id(&tes, &candidate).unwrap(),
            None
        );
    }
}
