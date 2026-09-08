use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::params::{
    EnablePhoenixMOESource, EnablePhoenixSource, EnableSimclustersSource, EnableTweetMixerSource,
};
use xai_candidate_pipeline::filter::{Filter, FilterResult};
use xai_home_mixer_proto::ServedType;

pub struct OONNsfwSimclustersFilter;

fn is_oon_retrieval(served_type: Option<ServedType>) -> bool {
    matches!(
        served_type,
        Some(ServedType::ForYouSimclusters)
            | Some(ServedType::ForYouPhoenixRetrieval)
            | Some(ServedType::ForYouPhoenixRetrievalMoe)
            | Some(ServedType::ForYouTweetMixer)
    )
}

impl Filter<ScoredPostsQuery, PostCandidate> for OONNsfwSimclustersFilter {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        !query.in_network_only
            && (query.params.get(EnableSimclustersSource)
                || query.params.get(EnablePhoenixSource)
                || query.params.get(EnablePhoenixMOESource)
                || query.params.get(EnableTweetMixerSource)
                || query.is_topic_request())
    }

    fn filter(
        &self,
        _query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let (removed, kept): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| {
            is_oon_retrieval(c.served_type)
                && c.in_network == Some(false)
                && c.nsfw_author == Some(true)
        });

        FilterResult { kept, removed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_home_mixer_proto::ServedType;

    fn candidate(
        tweet_id: u64,
        served_type: Option<ServedType>,
        in_network: Option<bool>,
        nsfw_author: Option<bool>,
    ) -> PostCandidate {
        PostCandidate {
            tweet_id,
            served_type,
            in_network,
            nsfw_author,
            ..Default::default()
        }
    }

    fn query_with_flags(flags: &[(&str, &str)]) -> ScoredPostsQuery {
        let mut query = ScoredPostsQuery::default();
        let fs = xai_feature_switches::FeatureSwitches::new(vec![]).unwrap();
        let mut results =
            fs.match_recipient(&xai_feature_switches::RecipientBuilder::new().build());
        for (key, value) in flags {
            results.override_fs(key.to_string(), value);
        }
        query.params = results.into();
        query
    }

    #[test]
    fn drops_simclusters_phoenix_moe_and_tweet_mixer_nsfw_authors() {
        let filter = OONNsfwSimclustersFilter;
        let query = ScoredPostsQuery::default();
        let candidates = vec![
            candidate(
                1,
                Some(ServedType::ForYouSimclusters),
                Some(false),
                Some(true),
            ),
            candidate(
                2,
                Some(ServedType::ForYouPhoenixRetrieval),
                Some(false),
                Some(true),
            ),
            candidate(
                3,
                Some(ServedType::ForYouPhoenixRetrievalMoe),
                Some(false),
                Some(true),
            ),
            candidate(
                4,
                Some(ServedType::ForYouTweetMixer),
                Some(false),
                Some(true),
            ),
            candidate(
                5,
                Some(ServedType::ForYouInNetwork),
                Some(true),
                Some(true),
            ),
            candidate(
                6,
                Some(ServedType::ForYouPhoenixRetrieval),
                Some(true),
                Some(true),
            ),
            candidate(
                7,
                Some(ServedType::ForYouPhoenixRetrieval),
                Some(false),
                Some(false),
            ),
            candidate(
                8,
                Some(ServedType::ForYouPhoenixRetrieval),
                Some(false),
                None,
            ),
        ];

        let result = filter.filter(&query, candidates);
        let removed: Vec<u64> = result.removed.iter().map(|c| c.tweet_id).collect();
        let kept: Vec<u64> = result.kept.iter().map(|c| c.tweet_id).collect();
        assert_eq!(removed, vec![1, 2, 3, 4]);
        assert_eq!(kept, vec![5, 6, 7, 8]);
    }

    #[test]
    fn enable_when_phoenix_on_and_simclusters_off() {
        let filter = OONNsfwSimclustersFilter;
        let query = query_with_flags(&[
            ("rust_home_mixer_enable_simclusters_source", "false"),
            ("rust_home_mixer_enable_phoenix_source", "true"),
            ("rust_home_mixer_enable_phoenix_moe_source", "false"),
            ("rust_home_mixer_enable_tweet_mixer_source", "false"),
        ]);
        assert!(filter.enable(&query));
    }

    #[test]
    fn disable_on_in_network_only() {
        let filter = OONNsfwSimclustersFilter;
        let mut query = query_with_flags(&[("rust_home_mixer_enable_phoenix_source", "true")]);
        query.in_network_only = true;
        assert!(!filter.enable(&query));
    }

    #[test]
    fn enable_on_topic_request_when_other_sources_off() {
        let filter = OONNsfwSimclustersFilter;
        let mut query = query_with_flags(&[
            ("rust_home_mixer_enable_simclusters_source", "false"),
            ("rust_home_mixer_enable_phoenix_source", "false"),
            ("rust_home_mixer_enable_phoenix_moe_source", "false"),
            ("rust_home_mixer_enable_tweet_mixer_source", "false"),
        ]);
        query.topic_ids = vec![42];
        assert!(filter.enable(&query));
    }
}
