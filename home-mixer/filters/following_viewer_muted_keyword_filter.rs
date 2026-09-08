use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::sync::Arc;
use xai_candidate_pipeline::filter::{Filter, FilterResult};
use xai_post_text::{MatchTweetGroup, TokenSequence, TweetTokenizer, UserMutes};

pub struct FollowingViewerMutedKeywordFilter {
    pub tokenizer: Arc<TweetTokenizer>,
}

impl FollowingViewerMutedKeywordFilter {
    pub fn new() -> Self {
        Self {
            tokenizer: Arc::new(TweetTokenizer::new()),
        }
    }
}

impl Filter<ScoredPostsQuery, PostCandidate> for FollowingViewerMutedKeywordFilter {
    fn filter(
        &self,
        query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let muted_keywords = &query.user_features.muted_keywords;
        if muted_keywords.is_empty() {
            return FilterResult {
                kept: candidates,
                removed: vec![],
            };
        }

        let tokenizer = Arc::clone(&self.tokenizer);
        tokio::task::block_in_place(|| {
            let token_sequences: Vec<TokenSequence> = muted_keywords
                .iter()
                .map(|k| tokenizer.tokenize(k))
                .collect();
            let matcher = MatchTweetGroup::new(UserMutes::new(token_sequences));

            let mut kept = Vec::new();
            let mut removed = Vec::new();
            for candidate in candidates {
                if candidate_matches(&candidate, &tokenizer, &matcher) {
                    removed.push(candidate);
                } else {
                    kept.push(candidate);
                }
            }
            FilterResult { kept, removed }
        })
    }
}

fn candidate_matches(
    candidate: &PostCandidate,
    tokenizer: &TweetTokenizer,
    matcher: &MatchTweetGroup,
) -> bool {
    std::iter::once(candidate.tweet_text.as_str())
        .chain(candidate.quoted_tweet_text.as_deref())
        .chain(candidate.retweeted_screen_name.as_deref())
        .chain(candidate.ancestor_texts.values().map(String::as_str))
        .filter(|text| !text.is_empty())
        .any(|text| matcher.matches(&tokenizer.tokenize(text)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_features::UserFeatures;

    fn query(muted_keywords: Vec<String>) -> ScoredPostsQuery {
        ScoredPostsQuery {
            user_features: UserFeatures {
                muted_keywords,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn text_candidate(tweet_id: u64, tweet_text: &str) -> PostCandidate {
        PostCandidate {
            tweet_id,
            tweet_text: tweet_text.to_string(),
            author_id: 12345,
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drops_retweet_when_original_author_handle_matches_muted_keyword() {
        let filter = FollowingViewerMutedKeywordFilter::new();
        let retweet = PostCandidate {
            tweet_id: 1,
            tweet_text: "ordinary news".to_string(),
            retweeted_tweet_id: Some(88),
            retweeted_user_id: Some(99),
            retweeted_screen_name: Some("spamaccount".to_string()),
            author_id: 12345,
            ..Default::default()
        };

        let result = filter.filter(
            &query(vec!["spamaccount".to_string()]),
            vec![retweet, text_candidate(2, "ordinary news")],
        );

        assert_eq!(result.kept.len(), 1);
        assert_eq!(result.kept[0].tweet_id, 2);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].tweet_id, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn keeps_retweet_when_original_author_handle_does_not_match() {
        let filter = FollowingViewerMutedKeywordFilter::new();
        let retweet = PostCandidate {
            tweet_id: 1,
            tweet_text: "ordinary news".to_string(),
            retweeted_tweet_id: Some(88),
            retweeted_user_id: Some(99),
            retweeted_screen_name: Some("normaluser".to_string()),
            author_id: 12345,
            ..Default::default()
        };

        let result = filter.filter(&query(vec!["spamaccount".to_string()]), vec![retweet]);

        assert_eq!(result.kept.len(), 1);
        assert!(result.removed.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn still_drops_when_quoted_text_matches() {
        let filter = FollowingViewerMutedKeywordFilter::new();
        let quote = PostCandidate {
            tweet_id: 1,
            tweet_text: "sharing this".to_string(),
            quoted_tweet_text: Some("this is spam content".to_string()),
            retweeted_screen_name: Some("normaluser".to_string()),
            author_id: 12345,
            ..Default::default()
        };

        let result = filter.filter(&query(vec!["spam".to_string()]), vec![quote]);

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].tweet_id, 1);
    }
}
