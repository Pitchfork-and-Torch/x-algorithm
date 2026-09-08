use crate::clients::tweet_entity_service_client::TESClient;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::utils::{QuickCache, default_quick_cache};
use xai_candidate_pipeline::hydrator::{CacheStore, CachedHydrator};
use xai_core_entities::entities::PureCoreData;
use xai_stats_receiver::global_stats_receiver;

const FOUND_SCOPE: [(&str, &str); 1] = [("hydration", "found")];
const MISSING_SCOPE: [(&str, &str); 1] = [("hydration", "missing")];

#[derive(Clone)]
pub struct CoreDataCandidateHydrator {
    pub tes_client: Arc<dyn TESClient + Send + Sync>,
    pub cache: Arc<QuickCache<u64, CoreDataCacheValue>>,
}

impl CoreDataCandidateHydrator {
    pub async fn new(tes_client: Arc<dyn TESClient + Send + Sync>) -> Self {
        Self {
            tes_client,
            cache: Arc::new(default_quick_cache()),
        }
    }
}

#[async_trait]
impl CachedHydrator<ScoredPostsQuery, PostCandidate> for CoreDataCandidateHydrator {
    type CacheKey = u64;

    type CacheValue = CoreDataCacheValue;

    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        !query.has_cached_posts
    }

    fn already_hydrated(&self, candidate: &PostCandidate) -> bool {
        if candidate.author_id == 0 || candidate.tweet_text.is_empty() {
            return false;
        }
        // NightOwl (Latest Following) pre-fills author_id and tweet_text.
        // Replies still need TES for ancestor_users (SelfReplyChainFilter).
        // Retweets still need TES for retweeted_user_id.
        if candidate.in_reply_to_tweet_id.is_some() && candidate.ancestor_users.is_empty() {
            return false;
        }
        if candidate.retweeted_tweet_id.is_some() && candidate.retweeted_user_id.is_none() {
            return false;
        }
        true
    }

    fn cache_store(&self) -> &dyn CacheStore<Self::CacheKey, Self::CacheValue> {
        self.cache.as_ref()
    }
    fn cache_key(&self, candidate: &PostCandidate) -> Self::CacheKey {
        candidate.tweet_id
    }

    fn cache_value(&self, hydrated: &PostCandidate) -> Self::CacheValue {
        CoreDataCacheValue {
            author_id: hydrated.author_id,
            retweeted_user_id: hydrated.retweeted_user_id,
            retweeted_tweet_id: hydrated.retweeted_tweet_id,
            in_reply_to_tweet_id: hydrated.in_reply_to_tweet_id,
            ancestor_users: hydrated.ancestor_users.clone(),
            tweet_text: hydrated.tweet_text.clone(),
        }
    }

    fn hydrate_from_cache(&self, value: Self::CacheValue) -> PostCandidate {
        PostCandidate {
            author_id: value.author_id,
            retweeted_user_id: value.retweeted_user_id,
            retweeted_tweet_id: value.retweeted_tweet_id,
            in_reply_to_tweet_id: value.in_reply_to_tweet_id,
            ancestor_users: value.ancestor_users,
            tweet_text: value.tweet_text,
            ..Default::default()
        }
    }

    async fn hydrate_from_client(
        &self,
        _query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let client = &self.tes_client;

        let post_features = client
            .get_tweet_core_datas(core_data_fetch_ids(candidates))
            .await;

        let mut hydrated_candidates = Vec::with_capacity(candidates.len());
        let mut hydrated_count = 0usize;
        let mut missing_count = 0usize;
        for candidate in candidates {
            match post_features.get(&candidate.tweet_id) {
                Some(Ok(Some(core_data))) => {
                    hydrated_count += 1;
                    let text = core_data.text.clone();
                    let ancestor_users = build_ancestor_users(candidate, core_data, &post_features);
                    let hydrated = PostCandidate {
                        author_id: core_data.author_id,
                        retweeted_user_id: core_data.source_user_id,
                        retweeted_tweet_id: core_data.source_tweet_id,
                        in_reply_to_tweet_id: core_data.in_reply_to_tweet_id,
                        ancestor_users,
                        tweet_text: text,
                        ..Default::default()
                    };
                    hydrated_candidates.push(Ok(hydrated));
                }
                Some(Ok(None)) | None => {
                    missing_count += 1;
                    // Keep source-populated fields. An empty default would
                    // clear NightOwl reply/retweet ids now that we call TES
                    // for those cards.
                    hydrated_candidates.push(Ok(source_core_fields(candidate)));
                }
                Some(Err(err)) => {
                    hydrated_candidates.push(Err(err.to_string()));
                }
            }
        }

        self.record_hydration_stats(hydrated_count, missing_count);

        hydrated_candidates
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        if candidate.author_id == 0 && hydrated.author_id != 0 {
            candidate.author_id = hydrated.author_id;
        }
        candidate.retweeted_user_id = hydrated.retweeted_user_id;
        candidate.retweeted_tweet_id = hydrated.retweeted_tweet_id;
        candidate.in_reply_to_tweet_id = hydrated.in_reply_to_tweet_id;
        candidate.ancestor_users = hydrated.ancestor_users;
        candidate.tweet_text = hydrated.tweet_text;
    }
}

fn source_core_fields(candidate: &PostCandidate) -> PostCandidate {
    PostCandidate {
        author_id: candidate.author_id,
        retweeted_user_id: candidate.retweeted_user_id,
        retweeted_tweet_id: candidate.retweeted_tweet_id,
        in_reply_to_tweet_id: candidate.in_reply_to_tweet_id,
        ancestor_users: candidate.ancestor_users.clone(),
        tweet_text: candidate.tweet_text.clone(),
        ..Default::default()
    }
}

fn core_data_fetch_ids(candidates: &[PostCandidate]) -> Vec<u64> {
    let mut fetch_ids: Vec<u64> = candidates.iter().map(|c| c.tweet_id).collect();
    fetch_ids.extend(
        candidates
            .iter()
            .filter(|c| c.ancestors.len() == 2)
            .map(|c| c.ancestors[1]),
    );
    fetch_ids
}

fn build_ancestor_users(
    candidate: &PostCandidate,
    core_data: &PureCoreData,
    core_datas: &HashMap<u64, anyhow::Result<Option<PureCoreData>>>,
) -> Vec<u64> {
    let mut ancestor_users = Vec::with_capacity(candidate.ancestors.len());
    if !candidate.ancestors.is_empty()
        && let Some(parent_author) = core_data.in_reply_to_user_id
    {
        ancestor_users.push(parent_author);
    }
    if candidate.ancestors.len() == 2
        && let Some(Ok(Some(root))) = core_datas.get(&candidate.ancestors[1])
    {
        ancestor_users.push(root.author_id);
    }
    ancestor_users
}

#[derive(Clone, Debug)]
pub struct CoreDataCacheValue {
    pub author_id: u64,
    pub retweeted_user_id: Option<u64>,
    pub retweeted_tweet_id: Option<u64>,
    pub in_reply_to_tweet_id: Option<u64>,
    pub ancestor_users: Vec<u64>,
    pub tweet_text: String,
}

impl CoreDataCandidateHydrator {
    fn record_hydration_stats(&self, hydrated_count: usize, missing_count: usize) {
        if let Some(receiver) = global_stats_receiver() {
            let metric_name = format!("{}.hydrate", self.name());
            receiver.incr(metric_name.as_str(), &FOUND_SCOPE, hydrated_count as u64);
            receiver.incr(metric_name.as_str(), &MISSING_SCOPE, missing_count as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::tweet_entity_service_client::MockTESClient;
    use crate::filters::self_reply_chain_filter::SelfReplyChainFilter;
    use crate::models::user_features::UserFeatures;
    use xai_candidate_pipeline::filter::Filter;
    use xai_candidate_pipeline::hydrator::Hydrator;

    fn nightowl_reply() -> PostCandidate {
        PostCandidate {
            tweet_id: 30,
            author_id: 10,
            in_reply_to_tweet_id: Some(20),
            ancestors: vec![20, 50],
            tweet_text: "self reply in someone else's thread".into(),
            ..Default::default()
        }
    }

    fn nightowl_retweet() -> PostCandidate {
        PostCandidate {
            tweet_id: 31,
            author_id: 10,
            retweeted_tweet_id: Some(21),
            tweet_text: "rt".into(),
            ..Default::default()
        }
    }

    fn nightowl_original() -> PostCandidate {
        PostCandidate {
            tweet_id: 32,
            author_id: 10,
            tweet_text: "hello".into(),
            ..Default::default()
        }
    }

    fn tes_reply() -> PureCoreData {
        PureCoreData {
            author_id: 10,
            text: "self reply in someone else's thread".into(),
            in_reply_to_tweet_id: Some(20),
            in_reply_to_user_id: Some(10),
            ..Default::default()
        }
    }

    fn tes_root() -> PureCoreData {
        PureCoreData {
            author_id: 99,
            text: "root from unfollowed user".into(),
            ..Default::default()
        }
    }

    fn tes_retweet() -> PureCoreData {
        PureCoreData {
            author_id: 10,
            text: "rt".into(),
            source_tweet_id: Some(21),
            source_user_id: Some(77),
            ..Default::default()
        }
    }

    async fn hydrator(core_data: HashMap<u64, Option<PureCoreData>>) -> CoreDataCandidateHydrator {
        CoreDataCandidateHydrator::new(Arc::new(MockTESClient {
            core_data,
            ..Default::default()
        }) as Arc<dyn TESClient + Send + Sync>)
        .await
    }

    async fn run(
        hydrator: &CoreDataCandidateHydrator,
        candidates: Vec<PostCandidate>,
    ) -> Vec<PostCandidate> {
        let mut candidates = candidates;
        let hydrated = Hydrator::hydrate(hydrator, &ScoredPostsQuery::default(), &candidates).await;
        hydrator.update_all(&mut candidates, hydrated);
        candidates
    }

    #[tokio::test]
    async fn already_hydrated_skips_complete_originals() {
        let h = hydrator(HashMap::new()).await;
        assert!(h.already_hydrated(&nightowl_original()));
        assert!(!h.already_hydrated(&nightowl_reply()));
        assert!(!h.already_hydrated(&nightowl_retweet()));
        assert!(!h.already_hydrated(&PostCandidate {
            author_id: 10,
            tweet_text: String::new(),
            ..Default::default()
        }));
    }

    #[tokio::test]
    async fn nightowl_reply_gets_ancestor_users_from_tes() {
        let mut core_data = HashMap::new();
        core_data.insert(30, Some(tes_reply()));
        core_data.insert(50, Some(tes_root()));
        let h = hydrator(core_data).await;

        let out = run(&h, vec![nightowl_reply()]).await;
        assert_eq!(out[0].ancestor_users, vec![10, 99]);
        assert_eq!(out[0].in_reply_to_tweet_id, Some(20));
        assert_eq!(out[0].author_id, 10);
        assert_eq!(out[0].tweet_text, "self reply in someone else's thread");
    }

    #[tokio::test]
    async fn nightowl_retweet_gets_original_author_from_tes() {
        let mut core_data = HashMap::new();
        core_data.insert(31, Some(tes_retweet()));
        let h = hydrator(core_data).await;

        let out = run(&h, vec![nightowl_retweet()]).await;
        assert_eq!(out[0].retweeted_tweet_id, Some(21));
        assert_eq!(out[0].retweeted_user_id, Some(77));
    }

    #[tokio::test]
    async fn nightowl_original_still_skips_tes() {
        let mut core_data = HashMap::new();
        core_data.insert(
            32,
            Some(PureCoreData {
                author_id: 10,
                text: "tes text must not replace nightowl".into(),
                ..Default::default()
            }),
        );
        let h = hydrator(core_data).await;
        let out = run(&h, vec![nightowl_original()]).await;
        assert_eq!(out[0].tweet_text, "hello");
    }

    #[tokio::test]
    async fn tes_miss_keeps_nightowl_reply_ids() {
        let h = hydrator(HashMap::new()).await;
        let out = run(&h, vec![nightowl_reply()]).await;
        assert_eq!(out[0].in_reply_to_tweet_id, Some(20));
        assert_eq!(out[0].ancestors, vec![20, 50]);
        assert_eq!(out[0].author_id, 10);
        assert_eq!(out[0].tweet_text, "self reply in someone else's thread");
        assert!(out[0].ancestor_users.is_empty());
    }

    #[tokio::test]
    async fn self_reply_chain_drops_after_nightowl_tes_hydrate() {
        let mut core_data = HashMap::new();
        core_data.insert(30, Some(tes_reply()));
        core_data.insert(50, Some(tes_root()));
        let h = hydrator(core_data).await;
        let out = run(&h, vec![nightowl_reply()]).await;

        let query = ScoredPostsQuery {
            user_id: 1,
            user_features: UserFeatures {
                followed_user_ids: vec![10],
                ..Default::default()
            },
            ..Default::default()
        };
        let result = SelfReplyChainFilter.filter(&query, out);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].tweet_id, 30);
        assert!(result.kept.is_empty());
    }
}
