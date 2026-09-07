use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::HashSet;
use xai_candidate_pipeline::filter::{Filter, FilterResult};

pub struct IneligibleSubscriptionFilter;

impl Filter<ScoredPostsQuery, PostCandidate> for IneligibleSubscriptionFilter {
    fn filter(
        &self,
        query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let subscribed_user_ids: HashSet<u64> = query
            .user_features
            .subscribed_user_ids
            .iter()
            .map(|id| *id as u64)
            .collect();

        let (kept, removed): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|candidate| keep_candidate(query, candidate, &subscribed_user_ids));

        FilterResult { kept, removed }
    }
}

fn keep_candidate(
    query: &ScoredPostsQuery,
    candidate: &PostCandidate,
    subscribed_user_ids: &HashSet<u64>,
) -> bool {
    let Some(author_id) = candidate.subscription_author_id else {
        return true;
    };

    // Same exemption VF DropExclusiveTweetContentRule gives the conversation author.
    if author_id == query.user_id {
        return true;
    }

    // Query hydrators ignore errors, so a socialgraph miss leaves an empty list.
    // Treat that as unknown, not "subscribed to nobody", and let VF decide.
    if !query.subscribed_user_ids_hydrated {
        return true;
    }

    subscribed_user_ids.contains(&author_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(subscription_author_id: Option<u64>) -> PostCandidate {
        PostCandidate {
            subscription_author_id,
            ..Default::default()
        }
    }

    fn query(user_id: u64, subscribed: Vec<i64>, hydrated: bool) -> ScoredPostsQuery {
        let mut query = ScoredPostsQuery {
            user_id,
            subscribed_user_ids_hydrated: hydrated,
            ..Default::default()
        };
        query.user_features.subscribed_user_ids = subscribed;
        query
    }

    #[tokio::test]
    async fn keeps_only_subscribed_authors_when_list_hydrated() {
        let filter = IneligibleSubscriptionFilter;
        let query = query(99, vec![1, 2], true);
        let candidates = vec![candidate(Some(1)), candidate(Some(3)), candidate(None)];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 2);
        assert_eq!(result.removed.len(), 1);
        assert!(result
            .kept
            .iter()
            .any(|c| c.subscription_author_id == Some(1)));
        assert!(result
            .kept
            .iter()
            .any(|c| c.subscription_author_id.is_none()));
        assert!(result
            .removed
            .iter()
            .any(|c| c.subscription_author_id == Some(3)));
    }

    #[tokio::test]
    async fn keeps_exclusive_posts_when_viewer_is_conversation_author() {
        let filter = IneligibleSubscriptionFilter;
        let query = query(7, vec![1], true);
        let candidates = vec![candidate(Some(7)), candidate(Some(3))];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.kept[0].subscription_author_id, Some(7));
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].subscription_author_id, Some(3));
    }

    #[tokio::test]
    async fn keeps_exclusive_posts_when_subscription_list_was_not_hydrated() {
        let filter = IneligibleSubscriptionFilter;
        let query = query(99, vec![], false);
        let candidates = vec![candidate(Some(3)), candidate(Some(4)), candidate(None)];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 3);
        assert!(result.removed.is_empty());
    }

    #[tokio::test]
    async fn drops_exclusive_posts_when_hydrated_list_is_empty() {
        let filter = IneligibleSubscriptionFilter;
        let query = query(99, vec![], true);
        let candidates = vec![candidate(Some(3)), candidate(None)];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert!(result.kept[0].subscription_author_id.is_none());
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].subscription_author_id, Some(3));
    }

    #[tokio::test]
    async fn conversation_author_kept_even_when_list_missing() {
        let filter = IneligibleSubscriptionFilter;
        let query = query(7, vec![], false);
        let candidates = vec![candidate(Some(7))];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert!(result.removed.is_empty());
    }
}
