use crate::models::candidate::{CandidateHelpers, PostCandidate};
use crate::models::query::ScoredPostsQuery;
use rustc_hash::FxHashMap;
use xai_candidate_pipeline::filter::{Filter, FilterResult};

pub struct DedupConversationFilter;

impl Filter<ScoredPostsQuery, PostCandidate> for DedupConversationFilter {
    fn filter(
        &self,
        _query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let tweet_to_root = conversation_roots(&candidates);
        let mut kept: Vec<PostCandidate> = Vec::with_capacity(candidates.len());
        let mut removed: Vec<PostCandidate> = Vec::new();
        let mut best_per_convo: FxHashMap<u64, (usize, f64)> =
            FxHashMap::with_capacity_and_hasher(candidates.len(), Default::default());

        for candidate in candidates {
            let conversation_id = get_conversation_id(&candidate, &tweet_to_root);
            let score = candidate.score.unwrap_or(0.0);

            if let Some((kept_idx, best_score)) = best_per_convo.get_mut(&conversation_id) {
                if score > *best_score {
                    let previous = std::mem::replace(&mut kept[*kept_idx], candidate);
                    removed.push(previous);
                    *best_score = score;
                } else {
                    removed.push(candidate);
                }
            } else {
                let idx = kept.len();
                best_per_convo.insert(conversation_id, (idx, score));
                kept.push(candidate);
            }
        }

        FilterResult { kept, removed }
    }
}

fn conversation_roots(candidates: &[PostCandidate]) -> FxHashMap<u64, u64> {
    let mut tweet_to_root =
        FxHashMap::with_capacity_and_hasher(candidates.len(), Default::default());
    for candidate in candidates {
        let Some(root) = candidate.ancestors.iter().copied().min() else {
            continue;
        };
        record_root(&mut tweet_to_root, candidate.tweet_id, root);
        for &ancestor in &candidate.ancestors {
            record_root(&mut tweet_to_root, ancestor, root);
        }
    }
    tweet_to_root
}

fn record_root(tweet_to_root: &mut FxHashMap<u64, u64>, tweet_id: u64, root: u64) {
    tweet_to_root
        .entry(tweet_id)
        .and_modify(|existing| *existing = (*existing).min(root))
        .or_insert(root);
}

fn get_conversation_id(candidate: &PostCandidate, tweet_to_root: &FxHashMap<u64, u64>) -> u64 {
    if let Some(root) = candidate.ancestors.iter().copied().min() {
        return root;
    }
    let original = candidate.get_original_tweet_id();
    tweet_to_root.get(&original).copied().unwrap_or(original)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(tweet_id: u64, ancestors: Vec<u64>, score: Option<f64>) -> PostCandidate {
        PostCandidate {
            tweet_id,
            ancestors,
            score,
            ..Default::default()
        }
    }

    fn retweet(tweet_id: u64, retweeted_tweet_id: u64, score: Option<f64>) -> PostCandidate {
        PostCandidate {
            tweet_id,
            retweeted_tweet_id: Some(retweeted_tweet_id),
            score,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn keeps_highest_scored_per_conversation() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            candidate(10, vec![1], Some(0.5)),
            candidate(11, vec![2, 1], Some(0.9)),
            candidate(12, vec![1], Some(0.7)),
            candidate(20, vec![2], Some(0.3)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 2);
        assert_eq!(result.removed.len(), 2);
        assert!(result
            .kept
            .iter()
            .any(|c| c.score == Some(0.9) && c.tweet_id == 11));
        assert!(result.kept.iter().any(|c| c.tweet_id == 20));
    }

    #[tokio::test]
    async fn keeps_first_when_scores_are_equal() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            candidate(1, vec![42], Some(1.0)),
            candidate(2, vec![42], Some(1.0)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept[0].score, Some(1.0));
        assert_eq!(result.kept[0].tweet_id, 1);
    }

    #[tokio::test]
    async fn does_not_dedup_without_conversation_id() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            candidate(1, vec![], Some(1.0)),
            candidate(2, vec![], Some(2.0)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 2);
        assert_eq!(result.removed.len(), 0);
    }

    #[tokio::test]
    async fn dedups_retweet_with_reply_sharing_original_as_ancestor() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            retweet(100, 1, Some(0.5)),
            candidate(11, vec![2, 1], Some(0.9)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept[0].tweet_id, 11);
        assert_eq!(result.kept[0].score, Some(0.9));
    }

    #[tokio::test]
    async fn keeps_higher_scored_retweet_over_reply_in_same_conversation() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            candidate(11, vec![2, 1], Some(0.4)),
            retweet(100, 1, Some(0.8)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept[0].tweet_id, 100);
        assert_eq!(result.kept[0].score, Some(0.8));
    }

    #[tokio::test]
    async fn dedups_retweet_of_mid_thread_reply_with_that_reply() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            retweet(100, 11, Some(0.5)),
            candidate(11, vec![2, 1], Some(0.9)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept[0].tweet_id, 11);
        assert_eq!(result.kept[0].score, Some(0.9));
    }

    #[tokio::test]
    async fn dedups_retweet_of_mid_thread_reply_listed_as_sibling_ancestor() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            retweet(100, 11, Some(0.8)),
            candidate(12, vec![11, 1], Some(0.4)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept[0].tweet_id, 100);
        assert_eq!(result.kept[0].score, Some(0.8));
    }

    #[tokio::test]
    async fn keeps_unrelated_retweet_when_original_is_not_in_the_thread() {
        let filter = DedupConversationFilter;
        let query = ScoredPostsQuery::default();

        let candidates = vec![
            retweet(100, 99, Some(0.5)),
            candidate(11, vec![2, 1], Some(0.9)),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.kept.len(), 2);
        assert!(result.kept.iter().any(|c| c.tweet_id == 100));
        assert!(result.kept.iter().any(|c| c.tweet_id == 11));
    }
}
