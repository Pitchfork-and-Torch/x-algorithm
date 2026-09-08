use crate::clients::gizmoduck_client::GizmoduckClient;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::HashSet;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::hydrator::Hydrator;

pub struct QuotedAuthorScreenNameHydrator {
    pub gizmoduck_client: Arc<dyn GizmoduckClient + Send + Sync>,
}

impl QuotedAuthorScreenNameHydrator {
    pub fn new(gizmoduck_client: Arc<dyn GizmoduckClient + Send + Sync>) -> Self {
        Self { gizmoduck_client }
    }
}

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for QuotedAuthorScreenNameHydrator {
    async fn hydrate(
        &self,
        _query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let quoted_user_ids: Vec<i64> = candidates
            .iter()
            .filter_map(|c| c.quoted_user_id)
            .map(|id| id as i64)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let users = if quoted_user_ids.is_empty() {
            Default::default()
        } else {
            self.gizmoduck_client.get_users(quoted_user_ids).await
        };

        candidates
            .iter()
            .map(|candidate| {
                let quoted_user = candidate
                    .quoted_user_id
                    .and_then(|id| users.get(&(id as i64)));
                match quoted_user {
                    Some(Err(err)) => Err(err.to_string()),
                    Some(Ok(Some(user))) => Ok(PostCandidate {
                        quoted_screen_name: user
                            .user
                            .as_ref()
                            .map(|u| u.profile.screen_name.clone()),
                        ..Default::default()
                    }),
                    Some(Ok(None)) | None => Ok(PostCandidate {
                        quoted_screen_name: None,
                        ..Default::default()
                    }),
                }
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.quoted_screen_name = hydrated.quoted_screen_name;
    }
}
