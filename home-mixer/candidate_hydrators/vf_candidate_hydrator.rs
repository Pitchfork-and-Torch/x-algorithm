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
use xai_visibility_filtering::vf_client::VfClient;

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
    ) -> HashMap<u64, Result<Option<FilteredReason>>> {
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
        let verdicts = VfVerdicts {
            in_network: in_network_result,
            oon: oon_result,
        };

        candidates
            .iter()
            .map(|candidate| resolve_visibility(candidate, &verdicts))
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.visibility_reason = hydrated.visibility_reason;
        candidate.drop_ancillary_posts = hydrated.drop_ancillary_posts;
    }
}

type VfResults = HashMap<u64, Result<Option<FilteredReason>>>;

/// VF verdicts kept separate by the safety level each id was evaluated under.
///
/// The same tweet id can legitimately be requested at both levels: an
/// in-network candidate may also be an ancestor or quoted post of another
/// candidate (evaluated as a recommendation), and an out-of-network candidate
/// may also be the source of a followed account's repost (evaluated as
/// in-network). Collapsing the two maps into one keyed by id alone lets one
/// verdict overwrite the other, so every lookup must go to the map that
/// matches how the id was bucketed above.
struct VfVerdicts {
    in_network: VfResults,
    oon: VfResults,
}

impl VfVerdicts {
    fn primary(&self, candidate: &PostCandidate) -> Option<&Result<Option<FilteredReason>>> {
        if candidate.in_network.unwrap_or(false) {
            self.in_network.get(&candidate.tweet_id)
        } else {
            self.oon.get(&candidate.tweet_id)
        }
    }
}

/// Same sentinel `XaiVfClient::results_to_map` already writes for a missing
/// response id. `VFFilter` hard-drops every non-`SafetyResult` reason, so this
/// reaches the filter. Returning `Err` does not: `Hydrator::update_all` skips
/// the write and leaves `visibility_reason = None`, which `VFFilter` keeps.
fn vf_lookup_unavailable() -> FilteredReason {
    FilteredReason::UnspecifiedReason
}

fn resolve_visibility(
    candidate: &PostCandidate,
    verdicts: &VfVerdicts,
) -> Result<PostCandidate, String> {
    let primary_result = verdicts.primary(candidate);
    // Ok(None) is a successful Allow. Err and a missing map key are not.
    let visibility_reason = match primary_result {
        Some(Ok(Some(reason))) => Some(reason.clone()),
        Some(Ok(None)) => None,
        Some(Err(_)) | None => Some(vf_lookup_unavailable()),
    };

    Ok(PostCandidate {
        visibility_reason,
        drop_ancillary_posts: Some(should_drop_ancillary(candidate, verdicts)),
        ..Default::default()
    })
}

fn ancillary_verdict_blocks(verdict: Option<&Result<Option<FilteredReason>>>) -> bool {
    match verdict {
        Some(Ok(Some(reason))) => should_drop_reason(reason),
        Some(Ok(None)) => false,
        Some(Err(_)) | None => true,
    }
}

fn should_drop_ancillary(candidate: &PostCandidate, verdicts: &VfVerdicts) -> bool {
    for &ancestor_id in &candidate.ancestors {
        if candidate.tombstone_ancestor_ids.contains(&ancestor_id) {
            continue;
        }
        if ancillary_verdict_blocks(verdicts.oon.get(&ancestor_id)) {
            return true;
        }
    }

    if let Some(quoted_id) = candidate.quoted_tweet_id
        && ancillary_verdict_blocks(verdicts.oon.get(&quoted_id))
    {
        return true;
    }

    if let Some(retweeted_id) = candidate.retweeted_tweet_id
        && ancillary_verdict_blocks(verdicts.in_network.get(&retweeted_id))
    {
        return true;
    }

    false
}

fn should_drop_reason(reason: &FilteredReason) -> bool {
    match reason {
        FilteredReason::SafetyResult(safety_result) => {
            matches!(safety_result.action, Action::Drop(_))
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use xai_visibility_filtering::models::SafetyResult;

    fn oon_only_drop() -> FilteredReason {
        FilteredReason::PossiblyUndesirable
    }

    fn interstitial() -> FilteredReason {
        FilteredReason::SafetyResult(SafetyResult {
            reason: None,
            action: Action::Interstitial,
        })
    }

    fn verdicts(
        in_network: Vec<(u64, Result<Option<FilteredReason>>)>,
        oon: Vec<(u64, Result<Option<FilteredReason>>)>,
    ) -> VfVerdicts {
        VfVerdicts {
            in_network: in_network.into_iter().collect(),
            oon: oon.into_iter().collect(),
        }
    }

    #[test]
    fn in_network_candidate_reads_timeline_home_verdict_when_id_is_also_an_oon_ancillary() {
        let verdicts = verdicts(
            vec![(1, Ok(None))],
            vec![(1, Ok(Some(oon_only_drop()))), (2, Ok(None))],
        );
        let in_network_post = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ..Default::default()
        };

        let hydrated = resolve_visibility(&in_network_post, &verdicts).unwrap();

        assert_eq!(hydrated.visibility_reason, None);
        assert_eq!(hydrated.drop_ancillary_posts, Some(false));
    }

    #[test]
    fn oon_candidate_reads_recommendations_verdict_when_id_is_also_a_repost_source() {
        let verdicts = verdicts(vec![(1, Ok(None))], vec![(1, Ok(Some(oon_only_drop())))]);
        let oon_post = PostCandidate {
            tweet_id: 1,
            in_network: Some(false),
            ..Default::default()
        };

        let hydrated = resolve_visibility(&oon_post, &verdicts).unwrap();

        assert_eq!(hydrated.visibility_reason, Some(oon_only_drop()));
    }

    #[test]
    fn missing_in_network_flag_is_treated_as_oon() {
        let verdicts = verdicts(vec![(1, Ok(None))], vec![(1, Ok(Some(oon_only_drop())))]);
        let post = PostCandidate {
            tweet_id: 1,
            in_network: None,
            ..Default::default()
        };

        let hydrated = resolve_visibility(&post, &verdicts).unwrap();

        assert_eq!(hydrated.visibility_reason, Some(oon_only_drop()));
    }

    #[test]
    fn ancestors_and_quoted_posts_use_recommendations_verdict() {
        let verdicts = verdicts(
            vec![(10, Ok(None)), (20, Ok(None))],
            vec![
                (10, Ok(Some(oon_only_drop()))),
                (20, Ok(Some(oon_only_drop()))),
            ],
        );

        let reply = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ancestors: vec![10],
            ..Default::default()
        };
        assert!(should_drop_ancillary(&reply, &verdicts));

        let quote = PostCandidate {
            tweet_id: 2,
            in_network: Some(true),
            quoted_tweet_id: Some(20),
            ..Default::default()
        };
        assert!(should_drop_ancillary(&quote, &verdicts));
    }

    #[test]
    fn repost_source_uses_timeline_home_verdict() {
        let verdicts = verdicts(vec![(10, Ok(None))], vec![(10, Ok(Some(oon_only_drop())))]);
        let repost = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            retweeted_tweet_id: Some(10),
            ..Default::default()
        };

        assert!(!should_drop_ancillary(&repost, &verdicts));

        let source_dropped_in_network = VfVerdicts {
            in_network: HashMap::from([(10, Ok(Some(oon_only_drop())))]),
            oon: HashMap::new(),
        };
        assert!(should_drop_ancillary(&repost, &source_dropped_in_network));
    }

    #[test]
    fn tombstoned_ancestors_are_skipped() {
        let verdicts = verdicts(vec![], vec![(10, Ok(Some(oon_only_drop())))]);
        let reply = PostCandidate {
            tweet_id: 1,
            ancestors: vec![10],
            tombstone_ancestor_ids: vec![10],
            ..Default::default()
        };

        assert!(!should_drop_ancillary(&reply, &verdicts));
    }

    #[test]
    fn interstitial_on_ancillary_does_not_drop() {
        let verdicts = verdicts(vec![], vec![(10, Ok(Some(interstitial())))]);
        let quote = PostCandidate {
            tweet_id: 1,
            quoted_tweet_id: Some(10),
            ..Default::default()
        };

        assert!(!should_drop_ancillary(&quote, &verdicts));
    }

    #[test]
    fn primary_lookup_error_stamps_unspecified_reason() {
        let verdicts = verdicts(vec![(1, Err(anyhow::anyhow!("vf unavailable")))], vec![]);
        let post = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ..Default::default()
        };

        let hydrated = resolve_visibility(&post, &verdicts).unwrap();

        assert_eq!(
            hydrated.visibility_reason,
            Some(FilteredReason::UnspecifiedReason)
        );
        assert_eq!(hydrated.drop_ancillary_posts, Some(false));
    }

    #[test]
    fn primary_missing_key_stamps_unspecified_reason() {
        let verdicts = verdicts(vec![], vec![]);
        let post = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ..Default::default()
        };

        let hydrated = resolve_visibility(&post, &verdicts).unwrap();

        assert_eq!(
            hydrated.visibility_reason,
            Some(FilteredReason::UnspecifiedReason)
        );
    }

    #[test]
    fn primary_allow_none_stays_none() {
        let verdicts = verdicts(vec![(1, Ok(None))], vec![]);
        let post = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ..Default::default()
        };

        let hydrated = resolve_visibility(&post, &verdicts).unwrap();

        assert_eq!(hydrated.visibility_reason, None);
        assert_eq!(hydrated.drop_ancillary_posts, Some(false));
    }

    #[test]
    fn ancillary_error_drops() {
        let verdicts = verdicts(
            vec![(1, Ok(None))],
            vec![(10, Err(anyhow::anyhow!("vf unavailable")))],
        );
        let quote = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            quoted_tweet_id: Some(10),
            ..Default::default()
        };

        assert!(should_drop_ancillary(&quote, &verdicts));
        let hydrated = resolve_visibility(&quote, &verdicts).unwrap();
        assert_eq!(hydrated.visibility_reason, None);
        assert_eq!(hydrated.drop_ancillary_posts, Some(true));
    }

    #[test]
    fn ancillary_missing_key_drops() {
        let verdicts = verdicts(vec![(1, Ok(None))], vec![]);
        let reply = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ancestors: vec![10],
            ..Default::default()
        };

        assert!(should_drop_ancillary(&reply, &verdicts));
    }

    #[test]
    fn ancillary_allow_none_does_not_drop() {
        let verdicts = verdicts(vec![(1, Ok(None))], vec![(10, Ok(None))]);
        let quote = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            quoted_tweet_id: Some(10),
            ..Default::default()
        };

        assert!(!should_drop_ancillary(&quote, &verdicts));
    }

    /// Answers Allow at TimelineHome and Drop at TimelineHomeRecommendations for
    /// every id, mimicking a post whose author carries an OON-only label.
    struct LevelSensitiveVfClient {
        calls: Mutex<Vec<(SafetyLevel, Vec<u64>)>>,
    }

    #[async_trait]
    impl VfClient for LevelSensitiveVfClient {
        async fn get_result(
            &self,
            post_ids: Vec<u64>,
            safety_level: SafetyLevel,
            _for_user_id: u64,
            _context: Option<TwitterContextViewer>,
        ) -> HashMap<u64, Result<Option<FilteredReason>>> {
            self.calls
                .lock()
                .unwrap()
                .push((safety_level.clone(), post_ids.clone()));
            let reason = match safety_level {
                TimelineHome => None,
                _ => Some(oon_only_drop()),
            };
            post_ids
                .into_iter()
                .map(|id| (id, Ok(reason.clone())))
                .collect()
        }
    }

    #[tokio::test]
    async fn followed_author_thread_root_is_not_dropped_because_reply_lists_it_as_ancestor() {
        let client = Arc::new(LevelSensitiveVfClient {
            calls: Mutex::new(Vec::new()),
        });
        let hydrator = VFCandidateHydrator::new(client.clone(), client.clone()).await;
        let root = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ..Default::default()
        };
        let reply_in_thread = PostCandidate {
            tweet_id: 2,
            in_network: Some(true),
            ancestors: vec![1],
            ..Default::default()
        };
        let quote_of_root = PostCandidate {
            tweet_id: 3,
            in_network: Some(false),
            quoted_tweet_id: Some(1),
            ..Default::default()
        };

        let results = hydrator
            .hydrate(
                &ScoredPostsQuery::default(),
                &[root, reply_in_thread, quote_of_root],
            )
            .await;

        let root = results[0].as_ref().unwrap();
        assert_eq!(
            root.visibility_reason, None,
            "in-network root must keep its TimelineHome verdict"
        );
        assert_eq!(root.drop_ancillary_posts, Some(false));

        let reply = results[1].as_ref().unwrap();
        assert_eq!(reply.visibility_reason, None);
        assert_eq!(
            reply.drop_ancillary_posts,
            Some(true),
            "ancestor is still judged as a recommendation"
        );

        let quote = results[2].as_ref().unwrap();
        assert_eq!(quote.visibility_reason, Some(oon_only_drop()));
        assert_eq!(quote.drop_ancillary_posts, Some(true));

        let calls = client.calls.lock().unwrap();
        let in_network_ids: Vec<u64> = calls
            .iter()
            .filter(|(level, _)| *level == TimelineHome)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        let oon_ids: Vec<u64> = calls
            .iter()
            .filter(|(level, _)| *level == TimelineHomeRecommendations)
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        assert!(in_network_ids.contains(&1) && oon_ids.contains(&1));
    }

    /// Returns Err for tweet 1, omits tweet 2, Allow for tweet 3.
    struct LookupMissVfClient;

    #[async_trait]
    impl VfClient for LookupMissVfClient {
        async fn get_result(
            &self,
            post_ids: Vec<u64>,
            _safety_level: SafetyLevel,
            _for_user_id: u64,
            _context: Option<TwitterContextViewer>,
        ) -> HashMap<u64, Result<Option<FilteredReason>>> {
            post_ids
                .into_iter()
                .filter_map(|id| match id {
                    1 => Some((id, Err(anyhow::anyhow!("vf unavailable")))),
                    2 => None,
                    _ => Some((id, Ok(None))),
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn hydrate_rpc_err_and_omitted_id_stamp_unspecified_allow_does_not() {
        let client = Arc::new(LookupMissVfClient);
        let hydrator = VFCandidateHydrator::new(client.clone(), client).await;
        let err_post = PostCandidate {
            tweet_id: 1,
            in_network: Some(true),
            ..Default::default()
        };
        let omitted_post = PostCandidate {
            tweet_id: 2,
            in_network: Some(true),
            ..Default::default()
        };
        let allow_post = PostCandidate {
            tweet_id: 3,
            in_network: Some(true),
            ..Default::default()
        };

        let results = hydrator
            .hydrate(
                &ScoredPostsQuery::default(),
                &[err_post, omitted_post, allow_post],
            )
            .await;

        assert_eq!(
            results[0].as_ref().unwrap().visibility_reason,
            Some(FilteredReason::UnspecifiedReason)
        );
        assert_eq!(
            results[1].as_ref().unwrap().visibility_reason,
            Some(FilteredReason::UnspecifiedReason)
        );
        assert_eq!(results[2].as_ref().unwrap().visibility_reason, None);
    }
}
