use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use xai_candidate_pipeline::filter::{Filter, FilterResult};

pub struct FollowingContentControlsFilter;

impl Filter<ScoredPostsQuery, PostCandidate> for FollowingContentControlsFilter {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        query.hides_replies() || query.hides_links() || query.hides_retweets()
    }

    fn filter(
        &self,
        query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let hide_replies = query.hides_replies();
        let hide_links = query.hides_links();
        let hide_retweets = query.hides_retweets();

        let (removed, kept): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| {
            (hide_replies && c.in_reply_to_tweet_id.is_some())
                || (hide_retweets && c.retweeted_tweet_id.is_some())
                || (hide_links && candidate_has_link(c))
        });

        FilterResult { kept, removed }
    }
}

fn candidate_has_link(candidate: &PostCandidate) -> bool {
    text_has_link(&candidate.tweet_text)
        || candidate
            .quoted_tweet_text
            .as_deref()
            .is_some_and(text_has_link)
        || candidate.ancestor_texts.values().any(|t| text_has_link(t))
}

fn text_has_link(text: &str) -> bool {
    text.contains("https://")
        || text.contains("http://")
        || text.contains("t.co/")
        || contains_www_host(text)
}

fn contains_www_host(text: &str) -> bool {
    text.match_indices("www.")
        .any(|(idx, _)| idx == 0 || !text.as_bytes()[idx - 1].is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        tweet_id: u64,
        in_reply_to_tweet_id: Option<u64>,
        retweeted_tweet_id: Option<u64>,
        tweet_text: &str,
    ) -> PostCandidate {
        PostCandidate {
            tweet_id,
            in_reply_to_tweet_id,
            retweeted_tweet_id,
            tweet_text: tweet_text.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn disabled_when_no_content_controls() {
        let query = ScoredPostsQuery::default();
        assert!(!FollowingContentControlsFilter.enable(&query));
    }

    #[test]
    fn enabled_for_each_viewer_preference() {
        for query in [
            ScoredPostsQuery {
                hide_replies: true,
                ..Default::default()
            },
            ScoredPostsQuery {
                exclude_replies: true,
                ..Default::default()
            },
            ScoredPostsQuery {
                hide_links: true,
                ..Default::default()
            },
            ScoredPostsQuery {
                exclude_retweets: true,
                ..Default::default()
            },
        ] {
            assert!(FollowingContentControlsFilter.enable(&query));
        }
    }

    #[test]
    fn hide_replies_drops_replies_keeps_originals() {
        let query = ScoredPostsQuery {
            hide_replies: true,
            ..Default::default()
        };
        let result = FollowingContentControlsFilter.filter(
            &query,
            vec![
                candidate(1, Some(10), None, "reply"),
                candidate(2, None, None, "original"),
                candidate(3, None, Some(30), "retweet"),
            ],
        );
        assert_eq!(
            result.kept.iter().map(|c| c.tweet_id).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            result
                .removed
                .iter()
                .map(|c| c.tweet_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn exclude_replies_alias_drops_replies() {
        let query = ScoredPostsQuery {
            exclude_replies: true,
            ..Default::default()
        };
        let result = FollowingContentControlsFilter.filter(
            &query,
            vec![candidate(1, Some(10), None, "reply")],
        );
        assert_eq!(result.removed[0].tweet_id, 1);
        assert!(result.kept.is_empty());
    }

    #[test]
    fn hide_links_drops_url_cards_and_quoted_links() {
        let query = ScoredPostsQuery {
            hide_links: true,
            ..Default::default()
        };
        let mut quoted = candidate(2, None, None, "look");
        quoted.quoted_tweet_text = Some("see https://example.com".to_string());
        let result = FollowingContentControlsFilter.filter(
            &query,
            vec![
                candidate(1, None, None, "plain text"),
                candidate(3, None, None, "watch https://t.co/abc"),
                quoted,
                candidate(4, None, None, "www.example.com"),
                candidate(5, None, None, "notwww.example"),
            ],
        );
        assert_eq!(
            result.kept.iter().map(|c| c.tweet_id).collect::<Vec<_>>(),
            vec![1, 5]
        );
        assert_eq!(
            result
                .removed
                .iter()
                .map(|c| c.tweet_id)
                .collect::<Vec<_>>(),
            vec![3, 2, 4]
        );
    }

    #[test]
    fn exclude_retweets_drops_retweets_keeps_replies() {
        let query = ScoredPostsQuery {
            exclude_retweets: true,
            ..Default::default()
        };
        let result = FollowingContentControlsFilter.filter(
            &query,
            vec![
                candidate(1, None, Some(10), "rt"),
                candidate(2, Some(20), None, "reply"),
                candidate(3, None, None, "original"),
            ],
        );
        assert_eq!(
            result.kept.iter().map(|c| c.tweet_id).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            result
                .removed
                .iter()
                .map(|c| c.tweet_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn off_flags_keep_every_type() {
        let query = ScoredPostsQuery::default();
        let candidates = vec![
            candidate(1, Some(10), None, "reply https://x.com"),
            candidate(2, None, Some(20), "rt"),
        ];
        let result = FollowingContentControlsFilter.filter(&query, candidates);
        assert_eq!(result.kept.len(), 2);
        assert!(result.removed.is_empty());
    }
}
