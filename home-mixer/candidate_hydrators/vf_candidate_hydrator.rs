use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::params::EnableXaiVfClient;
use anyhow::Result;
use futures::future::join;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::hydrator::Hydrator;
use xai_twittercontext_proto::GetTwitterContextViewer;
use xai_twittercontext_proto::TwitterContextViewer;
use xai_visibility_filtering::models::{Action, FilteredReason};
use xai_visibility_filtering::vf_client::SafetyLevel;
use xai_visibility_filtering::vf_client::SafetyLevel::{TimelineHome, TimelineHomeRecommendations};
use xai_visibility_filtering::vf_client::{TweetVisibility, VfClient};

pub struct VFCandidateHydrator {
    pub strato_vf_client: Arc<dyn VfClient + Send + Sync>,
    pub xai_vf_client: Arc<dyn VfClient + Send + Sync>,
}

impl VFCandidateHydrator {
    pub async fn new(
        strato_vf_client: Arc<dyn VfClient + Send + Sync>,
        xai_vf_client: Arc<dyn VfClient + Send + Sync>,
    ) -> Self {
        Self {
            strato_vf_client,
            xai_vf_client,
        }
    }

    async fn fetch_vf_results(
        client: &Arc<dyn VfClient + Send + Sync>,
        tweet_ids: Vec<u64>,
        safety_level: SafetyLevel,
        for_user_id: u64,
        context: Option<TwitterContextViewer>,
    ) -> HashMap<u64, Result<TweetVisibility>> {
        if tweet_ids.is_empty() {
            return HashMap::new();
        }

        client
            .get_result(tweet_ids, safety_level, for_user_id, context)
            .await
    }
}

#[async_trait]
impl Hydrator<ScoredPostsQuery, PostCandidate> for VFCandidateHydrator {
    async fn hydrate(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let context = query.get_viewer();
        let user_id = query.user_id;
        // Fully migrated to Rust VF. Old VF available in the event of production issues.
        let client = if query.params.get(EnableXaiVfClient) {
            &self.xai_vf_client
        } else {
            &self.strato_vf_client
        };

        let mut in_network_ids: Vec<u64> = Vec::new();
        let mut oon_ids: Vec<u64> = Vec::new();

        for candidate in candidates.iter() {
            if candidate.in_network.unwrap_or(false) {
                in_network_ids.push(candidate.tweet_id);
            } else {
                oon_ids.push(candidate.tweet_id);
            }
            for &ancestor_id in &candidate.ancestors {
                oon_ids.push(ancestor_id);
            }
            if let Some(quoted_id) = candidate.quoted_tweet_id {
                oon_ids.push(quoted_id);
            }
            if let Some(retweeted_id) = candidate.retweeted_tweet_id {
                in_network_ids.push(retweeted_id);
            }
        }

        in_network_ids.sort_unstable();
        in_network_ids.dedup();
        oon_ids.sort_unstable();
        oon_ids.dedup();

        let in_network_future = Self::fetch_vf_results(
            client,
            in_network_ids,
            TimelineHome,
            user_id,
            context.clone(),
        );

        let oon_future = Self::fetch_vf_results(
            client,
            oon_ids,
            TimelineHomeRecommendations,
            user_id,
            context,
        );

        let (in_network_result, oon_result) = join(in_network_future, oon_future).await;
        let mut all_results: HashMap<u64, Result<Option<FilteredReason>>> = HashMap::new();
        all_results.extend(
            oon_result
                .into_iter()
                .chain(in_network_result)
                .map(|(id, r)| (id, r.map(|t| t.reason))),
        );

        let mut hydrated_candidates = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let primary_result = all_results.get(&candidate.tweet_id);
            let visibility_reason = match primary_result {
                Some(Ok(Some(reason))) => Some(reason.clone()),
                _ => None,
            };

            let drop_ancillary = should_drop_ancillary(candidate, &all_results);

            let hydrated = match primary_result {
                Some(Err(err)) => Err(err.to_string()),
                _ => Ok(PostCandidate {
                    visibility_reason,
                    drop_ancillary_posts: Some(drop_ancillary),
                    ..Default::default()
                }),
            };
            hydrated_candidates.push(hydrated);
        }
        hydrated_candidates
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.visibility_reason = hydrated.visibility_reason;
        candidate.drop_ancillary_posts = hydrated.drop_ancillary_posts;
    }
}

pub(crate) fn should_drop_ancillary(
    candidate: &PostCandidate,
    vf_results: &HashMap<u64, Result<Option<FilteredReason>>>,
) -> bool {
    for &ancestor_id in &candidate.ancestors {
        if candidate.tombstone_ancestor_ids.contains(&ancestor_id) {
            continue;
        }
        if let Some(Ok(Some(reason))) = vf_results.get(&ancestor_id)
            && should_drop_reason(reason)
        {
            return true;
        }
    }

    if let Some(quoted_id) = candidate.quoted_tweet_id
        && let Some(Ok(Some(reason))) = vf_results.get(&quoted_id)
        && should_drop_reason(reason)
    {
        return true;
    }

    if let Some(retweeted_id) = candidate.retweeted_tweet_id
        && let Some(Ok(Some(reason))) = vf_results.get(&retweeted_id)
        && should_drop_reason(reason)
    {
        return true;
    }

    false
}

fn should_drop_reason(reason: &FilteredReason) -> bool {
    match reason {
        FilteredReason::SafetyResult(safety_result) => matches!(
            safety_result.action,
            // Keep-and-warn is only wired for the primary card (`visibility_reason`).
            // An ancillary Interstitial is discarded, so the wrapper would serve
            // NSFW/gore as a normal Allow embed. Drop the quote / RT / reply instead.
            Action::Drop(_) | Action::Interstitial
        ),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_visibility_filtering::models::SafetyResult;

    fn interstitial() -> FilteredReason {
        FilteredReason::SafetyResult(SafetyResult {
            reason: None,
            action: Action::Interstitial,
        })
    }

    fn allow() -> FilteredReason {
        FilteredReason::SafetyResult(SafetyResult {
            reason: None,
            action: Action::Allow,
        })
    }

    fn drop_action() -> FilteredReason {
        FilteredReason::SafetyResult(SafetyResult {
            reason: None,
            action: Action::Drop(Default::default()),
        })
    }

    fn results(pairs: Vec<(u64, FilteredReason)>) -> HashMap<u64, Result<Option<FilteredReason>>> {
        pairs
            .into_iter()
            .map(|(id, reason)| (id, Ok(Some(reason))))
            .collect()
    }

    #[test]
    fn interstitial_on_quoted_sets_drop_ancillary() {
        let quote = PostCandidate {
            tweet_id: 1,
            quoted_tweet_id: Some(10),
            ..Default::default()
        };
        assert!(should_drop_ancillary(&quote, &results(vec![(10, interstitial())])));
    }

    #[test]
    fn interstitial_on_retweet_source_sets_drop_ancillary() {
        let repost = PostCandidate {
            tweet_id: 1,
            retweeted_tweet_id: Some(10),
            ..Default::default()
        };
        assert!(should_drop_ancillary(
            &repost,
            &results(vec![(10, interstitial())])
        ));
    }

    #[test]
    fn interstitial_on_ancestor_sets_drop_ancillary() {
        let reply = PostCandidate {
            tweet_id: 1,
            ancestors: vec![10],
            ..Default::default()
        };
        assert!(should_drop_ancillary(
            &reply,
            &results(vec![(10, interstitial())])
        ));
    }

    #[test]
    fn tombstoned_ancestor_interstitial_is_skipped() {
        let reply = PostCandidate {
            tweet_id: 1,
            ancestors: vec![10],
            tombstone_ancestor_ids: vec![10],
            ..Default::default()
        };
        assert!(!should_drop_ancillary(
            &reply,
            &results(vec![(10, interstitial())])
        ));
    }

    #[test]
    fn allow_on_ancillary_does_not_drop() {
        let quote = PostCandidate {
            tweet_id: 1,
            quoted_tweet_id: Some(10),
            ..Default::default()
        };
        assert!(!should_drop_ancillary(&quote, &results(vec![(10, allow())])));
    }

    #[test]
    fn drop_on_ancillary_still_drops() {
        let quote = PostCandidate {
            tweet_id: 1,
            quoted_tweet_id: Some(10),
            ..Default::default()
        };
        assert!(should_drop_ancillary(
            &quote,
            &results(vec![(10, drop_action())])
        ));
    }

    #[test]
    fn primary_interstitial_without_ancillary_does_not_set_drop_flag() {
        let primary = PostCandidate {
            tweet_id: 1,
            ..Default::default()
        };
        assert!(!should_drop_ancillary(
            &primary,
            &results(vec![(1, interstitial())])
        ));
    }
}
