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

/// Soft VF verdicts on a quoted / ancestor / retweeted child must drop the
/// wrapper. Home Mixer has no interstitial, tombstone, or avoid chrome for
/// an embedded post, so keeping the card shows content soft-policy hid.
fn should_drop_reason(reason: &FilteredReason) -> bool {
    match reason {
        FilteredReason::SafetyResult(safety_result) => match safety_result.action {
            Action::Allow | Action::Downrank => false,
            Action::Drop(_)
            | Action::Avoid
            | Action::Tombstone
            | Action::Interstitial
            | Action::NotEvaluated => true,
        },
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safety(action: Action) -> FilteredReason {
        FilteredReason::SafetyResult(xai_visibility_filtering::models::SafetyResult {
            action,
            ..Default::default()
        })
    }

    fn results(
        entries: Vec<(u64, Result<Option<FilteredReason>>)>,
    ) -> HashMap<u64, Result<Option<FilteredReason>>> {
        entries.into_iter().collect()
    }

    fn quote_of(child_id: u64) -> PostCandidate {
        PostCandidate {
            tweet_id: 1,
            quoted_tweet_id: Some(child_id),
            ..Default::default()
        }
    }

    #[test]
    fn ancillary_soft_hide_drops_quote() {
        for action in [
            Action::Avoid,
            Action::Tombstone,
            Action::Interstitial,
            Action::NotEvaluated,
            Action::Drop(Default::default()),
        ] {
            let vf = results(vec![(1, Ok(None)), (10, Ok(Some(safety(action))))]);
            assert!(
                should_drop_ancillary(&quote_of(10), &vf),
                "ancillary {action:?} must drop the wrapper"
            );
        }
    }

    #[test]
    fn ancillary_allow_and_downrank_keep_quote() {
        for action in [Action::Allow, Action::Downrank] {
            let vf = results(vec![(1, Ok(None)), (10, Ok(Some(safety(action))))]);
            assert!(
                !should_drop_ancillary(&quote_of(10), &vf),
                "ancillary {action:?} must not drop the wrapper"
            );
        }

        let allow_none = results(vec![(1, Ok(None)), (10, Ok(None))]);
        assert!(!should_drop_ancillary(&quote_of(10), &allow_none));
    }

    #[test]
    fn ancillary_err_or_missing_still_fail_open_here() {
        // Primary/ancillary Err+miss is #121. This class is soft verdicts.
        let err = results(vec![(1, Ok(None)), (10, Err(anyhow::anyhow!("vf down")))]);
        assert!(!should_drop_ancillary(&quote_of(10), &err));

        let missing = results(vec![(1, Ok(None))]);
        assert!(!should_drop_ancillary(&quote_of(10), &missing));
    }

    #[test]
    fn interstitial_on_ancestor_drops_reply() {
        let vf = results(vec![
            (1, Ok(None)),
            (10, Ok(Some(safety(Action::Interstitial)))),
        ]);
        let reply = PostCandidate {
            tweet_id: 1,
            ancestors: vec![10],
            ..Default::default()
        };
        assert!(should_drop_ancillary(&reply, &vf));
    }

    #[test]
    fn tombstoned_ancestor_soft_hide_is_still_skipped() {
        let vf = results(vec![(10, Ok(Some(safety(Action::Interstitial))))]);
        let reply = PostCandidate {
            tweet_id: 1,
            ancestors: vec![10],
            tombstone_ancestor_ids: vec![10],
            ..Default::default()
        };
        assert!(!should_drop_ancillary(&reply, &vf));
    }
}
