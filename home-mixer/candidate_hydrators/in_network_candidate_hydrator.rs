use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::HashSet;
use tonic::async_trait;
use xai_candidate_pipeline::hydrator::Hydrator;

pub struct InNetworkCandidateHydrator;

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for InNetworkCandidateHydrator {
    // Always on. FollowedUserIdsQueryHydrator is request-fresh and this is a
    // local HashSet lookup. Skipping cache hits leaves Redis in_network (TTL
    // 180s) in place; VF then buckets TimelineHome vs Recs from that stamp.
    fn enable(&self, _query: &ScoredPostsQuery) -> bool {
        true
    }

    async fn hydrate(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let viewer_id = query.user_id;
        let followed_ids: HashSet<u64> = query
            .user_features
            .followed_user_ids
            .iter()
            .copied()
            .map(|id| id as u64)
            .collect();

        candidates
            .iter()
            .map(|candidate| {
                let is_self = candidate.author_id == viewer_id;
                let is_in_network = is_self || followed_ids.contains(&candidate.author_id);
                Ok(PostCandidate {
                    in_network: Some(is_in_network),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.in_network = hydrated.in_network;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_features::UserFeatures;

    fn query(viewer_id: u64, followed: Vec<i64>, has_cached_posts: bool) -> ScoredPostsQuery {
        ScoredPostsQuery {
            user_id: viewer_id,
            user_features: UserFeatures {
                followed_user_ids: followed,
                ..Default::default()
            },
            has_cached_posts,
            ..Default::default()
        }
    }

    async fn stamp(
        query: &ScoredPostsQuery,
        author_id: u64,
        cached_in_network: Option<bool>,
    ) -> Option<bool> {
        let hydrator = InNetworkCandidateHydrator;
        let candidates = vec![PostCandidate {
            tweet_id: 1,
            author_id,
            in_network: cached_in_network,
            ..Default::default()
        }];
        let hydrated = hydrator.hydrate(query, &candidates).await;
        let mut candidate = candidates.into_iter().next().unwrap();
        hydrator.update(&mut candidate, hydrated.into_iter().next().unwrap().unwrap());
        candidate.in_network
    }

    #[test]
    fn enable_stays_on_for_cached_posts() {
        let hydrator = InNetworkCandidateHydrator;
        assert!(hydrator.enable(&query(1, vec![42], true)));
        assert!(hydrator.enable(&query(1, vec![42], false)));
    }

    #[tokio::test]
    async fn restamps_stale_cached_in_network_after_unfollow() {
        let query = query(1, vec![], true);
        assert_eq!(
            stamp(&query, 42, Some(true)).await,
            Some(false),
            "cached Home stamp after unfollow must become OON"
        );
    }

    #[tokio::test]
    async fn restamps_stale_cached_oon_after_follow() {
        let query = query(1, vec![42], true);
        assert_eq!(
            stamp(&query, 42, Some(false)).await,
            Some(true),
            "cached Recs stamp after follow must become Home"
        );
    }

    #[tokio::test]
    async fn followed_author_is_in_network() {
        let query = query(1, vec![42], false);
        assert_eq!(stamp(&query, 42, None).await, Some(true));
    }

    #[tokio::test]
    async fn unfollowed_author_is_oon() {
        let query = query(1, vec![42], false);
        assert_eq!(stamp(&query, 99, None).await, Some(false));
    }
}
