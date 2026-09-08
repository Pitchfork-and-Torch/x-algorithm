use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::util::candidates_util::related_post_ids_iter;
use std::collections::HashSet;
use xai_candidate_pipeline::component_library::utils::BloomFilter;
use xai_candidate_pipeline::filter::{Filter, FilterResult};

pub struct PreviouslySeenPostsFilter;

impl Filter<ScoredPostsQuery, PostCandidate> for PreviouslySeenPostsFilter {
    fn filter(
        &self,
        query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let bloom_filters = query
            .bloom_filter_entries
            .iter()
            .map(|e| BloomFilter::from_parts(e.size_cap, e.false_positive_rate, &e.bloom_filter))
            .collect::<Vec<_>>();

        let seen_ids: HashSet<u64> = query.seen_ids.iter().copied().collect();

        let (removed, kept): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| {
            related_post_ids_iter(c).any(|post_id| {
                seen_ids.contains(&post_id)
                    || bloom_filters
                        .iter()
                        .any(|filter| filter.may_contain(post_id))
            })
        });

        FilterResult { kept, removed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(tweet_id: u64, retweeted: Option<u64>, in_reply_to: Option<u64>) -> PostCandidate {
        PostCandidate {
            tweet_id,
            retweeted_tweet_id: retweeted,
            in_reply_to_tweet_id: in_reply_to,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_reply_when_only_the_parent_was_seen() {
        let query = ScoredPostsQuery {
            seen_ids: vec![10],
            ..Default::default()
        };
        let result = PreviouslySeenPostsFilter.filter(
            &query,
            vec![
                candidate(20, None, Some(10)),
                candidate(10, None, None),
                candidate(30, Some(10), None),
            ],
        );
        let kept: Vec<u64> = result.kept.iter().map(|c| c.tweet_id).collect();
        let removed: Vec<u64> = result.removed.iter().map(|c| c.tweet_id).collect();
        assert_eq!(kept, vec![20]);
        assert_eq!(removed, vec![10, 30]);
    }

    #[test]
    fn drops_reply_when_the_reply_itself_was_seen() {
        let query = ScoredPostsQuery {
            seen_ids: vec![20],
            ..Default::default()
        };
        let result = PreviouslySeenPostsFilter.filter(&query, vec![candidate(20, None, Some(10))]);
        assert!(result.kept.is_empty());
        assert_eq!(result.removed.len(), 1);
    }
}
