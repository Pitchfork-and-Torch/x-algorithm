use crate::models::query::ScoredPostsQuery;
use crate::models::user_features::UserFeatures;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::clients::SocialGraphClientOps;
use xai_candidate_pipeline::query_hydrator::QueryHydrator;

pub struct SubscribedUserIdsQueryHydrator {
    pub socialgraph_client: Arc<dyn SocialGraphClientOps>,
}

#[async_trait]
impl QueryHydrator<ScoredPostsQuery> for SubscribedUserIdsQueryHydrator {
    async fn hydrate(&self, query: &ScoredPostsQuery) -> Result<ScoredPostsQuery, String> {
        let subscribed_user_ids = self
            .socialgraph_client
            .get_subscribed_user_ids(query.user_id)
            .await
            .map_err(|e| e.to_string())?;

        Ok(ScoredPostsQuery {
            user_features: UserFeatures {
                subscribed_user_ids,
                ..Default::default()
            },
            subscribed_user_ids_hydrated: true,
            ..Default::default()
        })
    }

    fn update(&self, query: &mut ScoredPostsQuery, hydrated: ScoredPostsQuery) {
        query.user_features.subscribed_user_ids = hydrated.user_features.subscribed_user_ids;
        query.subscribed_user_ids_hydrated = hydrated.subscribed_user_ids_hydrated;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_features::UserFeatures;
    use std::collections::HashSet;
    use tonic::Status;

    struct MockSocialGraph {
        ids: Result<Vec<i64>, Status>,
    }

    #[async_trait]
    impl SocialGraphClientOps for MockSocialGraph {
        async fn get_following_list(&self, _user_id: u64) -> Result<Vec<u64>, Status> {
            Ok(vec![])
        }
        async fn check_blocked_by(
            &self,
            _viewer_id: u64,
            _author_ids: &[u64],
        ) -> Result<HashSet<u64>, Status> {
            Ok(HashSet::new())
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
            match &self.ids {
                Ok(ids) => Ok(ids.clone()),
                Err(status) => Err(Status::new(status.code(), status.message())),
            }
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

    fn hydrator(ids: Result<Vec<i64>, Status>) -> SubscribedUserIdsQueryHydrator {
        SubscribedUserIdsQueryHydrator {
            socialgraph_client: Arc::new(MockSocialGraph { ids }),
        }
    }

    #[tokio::test]
    async fn successful_hydrate_marks_list_ready() {
        let hydrator = hydrator(Ok(vec![10, 20]));
        let result = hydrator.hydrate(&ScoredPostsQuery::default()).await.unwrap();
        assert_eq!(result.user_features.subscribed_user_ids, vec![10, 20]);
        assert!(result.subscribed_user_ids_hydrated);
    }

    #[tokio::test]
    async fn successful_empty_list_is_still_hydrated() {
        let hydrator = hydrator(Ok(vec![]));
        let result = hydrator.hydrate(&ScoredPostsQuery::default()).await.unwrap();
        assert!(result.user_features.subscribed_user_ids.is_empty());
        assert!(result.subscribed_user_ids_hydrated);
    }

    #[tokio::test]
    async fn socialgraph_error_does_not_mark_list_ready() {
        let hydrator = hydrator(Err(Status::unavailable("sg down")));
        assert!(hydrator.hydrate(&ScoredPostsQuery::default()).await.is_err());
    }

    #[test]
    fn update_copies_ids_and_hydrated_flag() {
        let hydrator = hydrator(Ok(vec![]));
        let mut query = ScoredPostsQuery::default();
        assert!(!query.subscribed_user_ids_hydrated);

        hydrator.update(
            &mut query,
            ScoredPostsQuery {
                user_features: UserFeatures {
                    subscribed_user_ids: vec![1, 2],
                    ..Default::default()
                },
                subscribed_user_ids_hydrated: true,
                ..Default::default()
            },
        );

        assert_eq!(query.user_features.subscribed_user_ids, vec![1, 2]);
        assert!(query.subscribed_user_ids_hydrated);
    }
}
