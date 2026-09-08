use crate::models::query::ScoredPostsQuery;
use crate::models::user_features::UserFeatures;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::clients::SocialGraphClientOps;
use xai_candidate_pipeline::query_hydrator::QueryHydrator;

pub struct MutedUserIdsQueryHydrator {
    pub socialgraph_client: Arc<dyn SocialGraphClientOps>,
}

#[async_trait]
impl QueryHydrator<ScoredPostsQuery> for MutedUserIdsQueryHydrator {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        !query.muted_user_ids_hydrated
    }

    async fn hydrate(&self, query: &ScoredPostsQuery) -> Result<ScoredPostsQuery, String> {
        let muted_user_ids = self
            .socialgraph_client
            .get_muted_user_ids(query.user_id)
            .await
            .map_err(|e| e.to_string())?;

        Ok(ScoredPostsQuery {
            user_features: UserFeatures {
                muted_user_ids,
                ..Default::default()
            },
            muted_user_ids_hydrated: true,
            ..Default::default()
        })
    }

    fn update(&self, query: &mut ScoredPostsQuery, hydrated: ScoredPostsQuery) {
        query.user_features.muted_user_ids = hydrated.user_features.muted_user_ids;
        query.muted_user_ids_hydrated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_candidate_pipeline::component_library::clients::MockSocialGraphClient;

    #[test]
    fn update_marks_list_ready() {
        let hydrator = MutedUserIdsQueryHydrator {
            socialgraph_client: Arc::new(MockSocialGraphClient),
        };
        let mut query = ScoredPostsQuery::default();
        let mut hydrated = ScoredPostsQuery::default();
        hydrated.user_features.muted_user_ids = vec![7];
        hydrator.update(&mut query, hydrated);
        assert_eq!(query.user_features.muted_user_ids, vec![7]);
        assert!(query.muted_user_ids_hydrated);
        assert!(!hydrator.enable(&query));
    }

    #[test]
    fn enable_when_list_not_loaded() {
        let hydrator = MutedUserIdsQueryHydrator {
            socialgraph_client: Arc::new(MockSocialGraphClient),
        };
        assert!(hydrator.enable(&ScoredPostsQuery::default()));
    }
}
