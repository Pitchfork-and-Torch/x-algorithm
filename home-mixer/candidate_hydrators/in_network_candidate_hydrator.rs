use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use tonic::async_trait;
use xai_candidate_pipeline::hydrator::Hydrator;

pub struct InNetworkCandidateHydrator;

fn stamp_in_network(query: &ScoredPostsQuery, author_id: u64) -> bool {
    let viewer_id = query.user_id;
    author_id == viewer_id
        || query
            .user_features
            .followed_user_ids
            .iter()
            .copied()
            .map(|id| id as u64)
            .any(|id| id == author_id)
}

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for InNetworkCandidateHydrator {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        !query.has_cached_posts
    }

    async fn hydrate(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        candidates
            .iter()
            .map(|candidate| {
                Ok(PostCandidate {
                    in_network: Some(stamp_in_network(query, candidate.author_id)),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.in_network = hydrated.in_network;
    }

    fn apply_hydration(
        &self,
        query: &ScoredPostsQuery,
        candidates: &mut [PostCandidate],
        _hydrated: Vec<Result<PostCandidate, String>>,
    ) {
        // run_hydrators join_alls hydrate() against the source snapshot, then
        // applies writes in vec order. CoreData.update may have just filled
        // author_id. Copying hydrate()'s snapshot stamp would keep OON on a
        // followee TweetMixer shipped as author_id = 0.
        for candidate in candidates.iter_mut() {
            candidate.in_network = Some(stamp_in_network(query, candidate.author_id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_features::UserFeatures;

    fn query_following(viewer_id: u64, followed: Vec<i64>) -> ScoredPostsQuery {
        ScoredPostsQuery {
            user_id: viewer_id,
            user_features: UserFeatures {
                followed_user_ids: followed,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn hydrate_stamps_from_snapshot_author() {
        let query = query_following(1, vec![42]);
        let hydrator = InNetworkCandidateHydrator;
        let candidates = vec![
            PostCandidate {
                tweet_id: 1,
                author_id: 42,
                ..Default::default()
            },
            PostCandidate {
                tweet_id: 2,
                author_id: 99,
                ..Default::default()
            },
            PostCandidate {
                tweet_id: 3,
                author_id: 1,
                ..Default::default()
            },
        ];
        let hydrated = hydrator.hydrate(&query, &candidates).await;
        assert_eq!(hydrated[0].as_ref().unwrap().in_network, Some(true));
        assert_eq!(hydrated[1].as_ref().unwrap().in_network, Some(false));
        assert_eq!(hydrated[2].as_ref().unwrap().in_network, Some(true));
    }

    #[tokio::test]
    async fn join_all_snapshot_stamp_is_oon_when_author_id_is_zero() {
        let query = query_following(1, vec![42]);
        let hydrator = InNetworkCandidateHydrator;
        let snapshot = [PostCandidate {
            tweet_id: 1,
            author_id: 0,
            ..Default::default()
        }];
        let hydrated = hydrator.hydrate(&query, &snapshot).await;
        let mut candidate = snapshot[0].clone();
        hydrator.update(&mut candidate, hydrated.into_iter().next().unwrap().unwrap());
        candidate.author_id = 42;
        assert_eq!(
            candidate.in_network,
            Some(false),
            "hydrate+update copies the pre-TES OON stamp; this is the join_all bug"
        );
    }

    #[tokio::test]
    async fn apply_hydration_restamps_after_tes_fills_followed_author() {
        let query = query_following(1, vec![42]);
        let hydrator = InNetworkCandidateHydrator;
        let snapshot = [PostCandidate {
            tweet_id: 1,
            author_id: 0,
            ..Default::default()
        }];
        let hydrated = hydrator.hydrate(&query, &snapshot).await;
        let mut candidates = snapshot.to_vec();
        // CoreData.update in the join_all write-back, before InNetwork.
        candidates[0].author_id = 42;
        hydrator.apply_hydration(&query, &mut candidates, hydrated);
        assert_eq!(candidates[0].in_network, Some(true));
    }

    #[tokio::test]
    async fn apply_hydration_keeps_oon_for_unfollowed_tes_author() {
        let query = query_following(1, vec![42]);
        let hydrator = InNetworkCandidateHydrator;
        let snapshot = [PostCandidate {
            tweet_id: 1,
            author_id: 0,
            in_network: Some(true),
            ..Default::default()
        }];
        let hydrated = hydrator.hydrate(&query, &snapshot).await;
        let mut candidates = snapshot.to_vec();
        candidates[0].author_id = 99;
        hydrator.apply_hydration(&query, &mut candidates, hydrated);
        assert_eq!(candidates[0].in_network, Some(false));
    }

    #[tokio::test]
    async fn apply_hydration_marks_self_in_network() {
        let query = query_following(7, vec![42]);
        let hydrator = InNetworkCandidateHydrator;
        let mut candidates = vec![PostCandidate {
            tweet_id: 1,
            author_id: 7,
            in_network: Some(false),
            ..Default::default()
        }];
        hydrator.apply_hydration(&query, &mut candidates, vec![]);
        assert_eq!(candidates[0].in_network, Some(true));
    }

    #[test]
    fn tes_miss_leaving_author_zero_stamps_oon() {
        let query = query_following(1, vec![42]);
        assert!(!stamp_in_network(&query, 0));
    }
}
