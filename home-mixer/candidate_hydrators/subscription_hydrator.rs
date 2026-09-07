use crate::clients::tweet_entity_service_client::TESClient;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::utils::{default_quick_cache, QuickCache};
use xai_candidate_pipeline::hydrator::{CacheStore, CachedHydrator};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionAuthorHydration {
    Resolved(Option<u64>),
    Failed,
}

pub struct SubscriptionHydrator {
    pub tes_client: Arc<dyn TESClient + Send + Sync>,
    pub cache: QuickCache<u64, SubscriptionAuthorHydration>,
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
    type CacheValue = SubscriptionAuthorHydration;

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
        cache_value(hydrated)
    }

    fn hydrate_from_cache(&self, value: Self::CacheValue) -> PostCandidate {
        hydrate_from_cache(value)
    }

    async fn hydrate_from_client(
        &self,
        _query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let client = &self.tes_client;

        let tweet_ids: Vec<u64> = candidates.iter().map(|c| c.tweet_id).collect();

        let post_features = client.get_subscription_author_ids(tweet_ids.clone()).await;

        let mut hydrated_candidates = Vec::with_capacity(candidates.len());
        for tweet_id in tweet_ids {
            hydrated_candidates.push(Ok(hydrate_from_tes(post_features.get(&tweet_id))));
        }

        hydrated_candidates
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.subscription_author_id = hydrated.subscription_author_id;
        candidate.subscription_lookup_failed = hydrated.subscription_lookup_failed;
    }
}

fn cache_value(hydrated: &PostCandidate) -> SubscriptionAuthorHydration {
    if hydrated.subscription_lookup_failed == Some(true) {
        SubscriptionAuthorHydration::Failed
    } else {
        SubscriptionAuthorHydration::Resolved(hydrated.subscription_author_id)
    }
}

fn hydrate_from_tes<E>(result: Option<&Result<Option<u64>, E>>) -> PostCandidate {
    match result {
        Some(Ok(value)) => PostCandidate {
            subscription_author_id: *value,
            subscription_lookup_failed: Some(false),
            ..Default::default()
        },
        None | Some(Err(_)) => PostCandidate {
            subscription_lookup_failed: Some(true),
            ..Default::default()
        },
    }
}

fn hydrate_from_cache(value: SubscriptionAuthorHydration) -> PostCandidate {
    match value {
        SubscriptionAuthorHydration::Failed => PostCandidate {
            subscription_lookup_failed: Some(true),
            ..Default::default()
        },
        SubscriptionAuthorHydration::Resolved(author_id) => PostCandidate {
            subscription_author_id: author_id,
            subscription_lookup_failed: Some(false),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trip_preserves_lookup_failure() {
        let failed = PostCandidate {
            subscription_lookup_failed: Some(true),
            ..Default::default()
        };
        let cached = cache_value(&failed);
        assert_eq!(cached, SubscriptionAuthorHydration::Failed);
        let restored = hydrate_from_cache(cached);
        assert_eq!(restored.subscription_lookup_failed, Some(true));
        assert!(restored.subscription_author_id.is_none());
    }

    #[test]
    fn tes_err_or_missing_slot_fails_closed() {
        let err: Result<Option<u64>, &str> = Err("tes unavailable");
        let failed = hydrate_from_tes(Some(&err));
        assert_eq!(failed.subscription_lookup_failed, Some(true));
        assert!(failed.subscription_author_id.is_none());

        let missing: Option<&Result<Option<u64>, &str>> = None;
        let missing = hydrate_from_tes(missing);
        assert_eq!(missing.subscription_lookup_failed, Some(true));

        let ok: Result<Option<u64>, &str> = Ok(None);
        let not_exclusive = hydrate_from_tes(Some(&ok));
        assert_eq!(not_exclusive.subscription_lookup_failed, Some(false));
        assert!(not_exclusive.subscription_author_id.is_none());

        let exclusive: Result<Option<u64>, &str> = Ok(Some(42));
        let exclusive = hydrate_from_tes(Some(&exclusive));
        assert_eq!(exclusive.subscription_lookup_failed, Some(false));
        assert_eq!(exclusive.subscription_author_id, Some(42));
    }

    #[test]
    fn cache_round_trip_preserves_not_exclusive() {
        let resolved = PostCandidate {
            subscription_author_id: None,
            subscription_lookup_failed: Some(false),
            ..Default::default()
        };
        let cached = cache_value(&resolved);
        assert_eq!(cached, SubscriptionAuthorHydration::Resolved(None));
        let restored = hydrate_from_cache(cached);
        assert_eq!(restored.subscription_lookup_failed, Some(false));
        assert!(restored.subscription_author_id.is_none());
    }
}
