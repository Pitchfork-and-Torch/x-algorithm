use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::clients::SocialGraphClientOps;
use xai_candidate_pipeline::hydrator::Hydrator;

pub struct BlockedByHydrator {
    pub socialgraph_client: Arc<dyn SocialGraphClientOps>,
}

impl BlockedByHydrator {
    pub async fn new(socialgraph_client: Arc<dyn SocialGraphClientOps>) -> Self {
        Self { socialgraph_client }
    }
}

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for BlockedByHydrator {
    fn enable(&self, _query: &ScoredPostsQuery) -> bool {
        // MutedUserIds / BlockedUserIds query hydrators still run on cache
        // hits. Skipping here keeps author_blocks_viewer from the Redis slate
        // (TTL 180s). A newly blocking author then still serves.
        true
    }

    async fn hydrate(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let author_ids: Vec<u64> = candidates.iter().map(|x| x.author_id).collect();

        let blocked_by_user_ids = match self
            .socialgraph_client
            .check_blocked_by(query.user_id, &author_ids)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                let err_msg = e.to_string();
                return candidates.iter().map(|_| Err(err_msg.clone())).collect();
            }
        };
        candidates
            .iter()
            .map(|candidate| {
                let author_blocks_viewer = blocked_by_user_ids.contains(&candidate.author_id);
                Ok(PostCandidate {
                    author_blocks_viewer: Some(author_blocks_viewer),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.author_blocks_viewer = hydrated.author_blocks_viewer;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tonic::Status;
    use xai_candidate_pipeline::component_library::clients::SocialGraphClientOps;

    struct MockSocialGraph {
        blocked_by: HashSet<u64>,
    }

    #[async_trait]
    impl SocialGraphClientOps for MockSocialGraph {
        async fn get_following_list(&self, _user_id: u64) -> Result<Vec<u64>, Status> {
            Ok(vec![])
        }
        async fn check_blocked_by(
            &self,
            _viewer_id: u64,
            author_ids: &[u64],
        ) -> Result<HashSet<u64>, Status> {
            Ok(author_ids
                .iter()
                .copied()
                .filter(|id| self.blocked_by.contains(id))
                .collect())
        }
        async fn check_followed_by(
            &self,
            _viewer_id: u64,
            _user_ids: &[u64],
        ) -> Result<HashSet<u64>, Status> {
            Ok(HashSet::new())
        }
        async fn get_blocked_user_ids(&self, _viewer_id: u64) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
        async fn get_muted_user_ids(&self, _viewer_id: u64) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
        async fn get_followed_user_ids(&self, _viewer_id: u64) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
        async fn get_follower_ids(&self, _user_id: u64) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
        async fn get_subscribed_user_ids(&self, _viewer_id: u64) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
        async fn get_device_following_user_ids(&self, _viewer_id: u64) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
        async fn get_hide_recommendations_user_ids(
            &self,
            _viewer_id: u64,
        ) -> Result<Vec<i64>, Status> {
            Ok(vec![])
        }
    }

    fn hydrator(blocked_by: &[u64]) -> BlockedByHydrator {
        BlockedByHydrator {
            socialgraph_client: Arc::new(MockSocialGraph {
                blocked_by: blocked_by.iter().copied().collect(),
            }),
        }
    }

    fn query(has_cached_posts: bool) -> ScoredPostsQuery {
        ScoredPostsQuery {
            user_id: 1,
            has_cached_posts,
            ..Default::default()
        }
    }

    fn candidate(tweet_id: u64, author_id: u64, author_blocks_viewer: Option<bool>) -> PostCandidate {
        PostCandidate {
            tweet_id,
            author_id,
            author_blocks_viewer,
            ..Default::default()
        }
    }

    #[test]
    fn enable_on_cache_hit_and_miss() {
        let hydrator = hydrator(&[]);
        assert!(hydrator.enable(&query(true)));
        assert!(hydrator.enable(&query(false)));
    }

    #[tokio::test]
    async fn cache_hit_recomputes_author_blocks_viewer() {
        let hydrator = hydrator(&[20]);
        let q = query(true);
        let mut candidates = vec![
            candidate(1, 10, Some(true)),
            candidate(2, 20, Some(false)),
            candidate(3, 30, None),
        ];

        let hydrated = hydrator.hydrate(&q, &candidates).await;
        for (c, h) in candidates.iter_mut().zip(hydrated) {
            hydrator.update(c, h.expect("hydrate ok"));
        }

        assert_eq!(candidates[0].author_blocks_viewer, Some(false));
        assert_eq!(candidates[1].author_blocks_viewer, Some(true));
        assert_eq!(candidates[2].author_blocks_viewer, Some(false));
    }
}
