use crate::clients::socialgraph_client::SocialgraphClient;
use crate::hydration::batch::{Hydrated, TweetHydrationBatch};
use crate::hydration::metrics::{record_batch_size, timed_results};
use crate::models::{ExclusiveContentFeatures, TweetId, Viewer};
use crate::rules::SafetyLevel;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use xai_core_entities::tweet_entity_service_client::TESClient;

const CLIENT_TIMEOUT: Duration = Duration::from_millis(150);
const CLIENT: &str = "exclusive_content";

pub struct ExclusiveContentHydrator {
    pub tes_client: Arc<dyn TESClient + Send + Sync>,
    pub sg_client: Arc<dyn SocialgraphClient + Send + Sync>,
}

impl ExclusiveContentHydrator {
    pub async fn hydrate(
        &self,
        tweet_ids: &[TweetId],
        viewer: Viewer,
        safety_level: SafetyLevel,
    ) -> TweetHydrationBatch<ExclusiveContentFeatures> {
        let candidate_count_by_key: HashMap<u64, usize> = tweet_ids
            .iter()
            .fold(HashMap::with_capacity(tweet_ids.len()), |mut counts, id| {
                *counts.entry(id.0).or_default() += 1;
                counts
            });
        record_batch_size(CLIENT, candidate_count_by_key.len());
        let raw_ids: Vec<u64> = candidate_count_by_key.keys().copied().collect();

        let exclusive_controls = timed_results(
            CLIENT,
            "get_exclusive_controls",
            safety_level,
            &candidate_count_by_key,
            CLIENT_TIMEOUT,
            self.tes_client.get_exclusive_controls(raw_ids),
        )
        .await
        .map_keys(TweetId);

        let root_author_ids: Vec<u64> = tweet_ids
            .iter()
            .filter_map(|id| {
                exclusive_controls
                    .get(id)
                    .map(|ctrl| ctrl.conversation_author_id as u64)
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let super_follows = match viewer.user_id() {
            Some(vid) if !root_author_ids.is_empty() => {
                timed_rpc_super_follows(
                    self.sg_client.as_ref(),
                    vid,
                    &root_author_ids,
                    safety_level,
                )
                .await
            }
            _ => HashMap::new(),
        };

        exclusive_controls.map(|ctrl| {
            let author_id = ctrl.conversation_author_id as u64;
            ExclusiveContentFeatures {
                conversation_author_id: author_id,
                viewer_super_follows_author: super_follows.get(&author_id).copied().unwrap_or(false),
            }
        })
    }
}

async fn timed_rpc_super_follows(
    sg_client: &(dyn SocialgraphClient + Send + Sync),
    viewer_id: u64,
    root_author_ids: &[u64],
    safety_level: SafetyLevel,
) -> HashMap<u64, bool> {
    use crate::hydration::metrics::{timed_rpc, HydratorOutcome};
    timed_rpc(
        CLIENT,
        "batch_check_super_follows",
        safety_level,
        root_author_ids.len(),
        CLIENT_TIMEOUT,
        |_| HydratorOutcome::Success,
        sg_client.batch_check_super_follows(viewer_id, root_author_ids),
    )
    .await
}

pub(crate) fn exclusive_slot(
    batch: &TweetHydrationBatch<ExclusiveContentFeatures>,
    tweet_id: &TweetId,
) -> (Option<ExclusiveContentFeatures>, bool) {
    match batch.hydrated(tweet_id) {
        Some(Hydrated::Found(features)) => (Some(features.clone()), false),
        Some(Hydrated::NotFound) => (None, false),
        Some(Hydrated::Failed(_)) | None => (None, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydration::batch::HydrationBatch;

    fn batch(
        results: HashMap<TweetId, Result<Option<ExclusiveContentFeatures>, &'static str>>,
    ) -> TweetHydrationBatch<ExclusiveContentFeatures> {
        let keys: Vec<TweetId> = results.keys().copied().collect();
        HydrationBatch::from_results(keys, results)
    }

    fn exclusive(author: u64) -> ExclusiveContentFeatures {
        ExclusiveContentFeatures {
            conversation_author_id: author,
            viewer_super_follows_author: false,
        }
    }

    #[test]
    fn found_exclusive_is_not_a_hydration_failure() {
        let batch = batch(HashMap::from([(TweetId(1), Ok(Some(exclusive(100))))]));
        let (features, failed) = exclusive_slot(&batch, &TweetId(1));
        assert_eq!(features.unwrap().conversation_author_id, 100);
        assert!(!failed);
    }

    #[test]
    fn not_found_means_not_exclusive() {
        let batch = batch(HashMap::from([(
            TweetId(1),
            Ok::<_, &str>(None),
        )]));
        let (features, failed) = exclusive_slot(&batch, &TweetId(1));
        assert!(features.is_none());
        assert!(!failed);
    }

    #[test]
    fn rpc_error_fails_closed() {
        let batch = batch(HashMap::from([(
            TweetId(1),
            Err("tes exclusive unavailable"),
        )]));
        let (features, failed) = exclusive_slot(&batch, &TweetId(1));
        assert!(features.is_none());
        assert!(failed);
    }

    #[test]
    fn missing_key_fails_closed() {
        let batch = batch(HashMap::from([(TweetId(1), Ok(Some(exclusive(100))))]));
        let (features, failed) = exclusive_slot(&batch, &TweetId(2));
        assert!(features.is_none());
        assert!(failed);
    }

    #[test]
    fn timeout_batch_fails_closed() {
        let batch: TweetHydrationBatch<ExclusiveContentFeatures> =
            HydrationBatch::timed_out([TweetId(1), TweetId(2)]);
        assert!(exclusive_slot(&batch, &TweetId(1)).1);
        assert!(exclusive_slot(&batch, &TweetId(2)).1);
    }
}
