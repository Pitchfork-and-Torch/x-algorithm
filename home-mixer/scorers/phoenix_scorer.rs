use crate::models::candidate::CandidateHelpers;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::params::{
    PhoenixInferenceClusterId, PhoenixRankerNewUserHistoryThreshold,
    PhoenixRankerNewUserInferenceClusterId, RerankerHeadTag,
};
use crate::util::egress::PredictionDispatch;
use crate::util::phoenix_request::build_prediction_request;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::clients::phoenix_prediction_client::PhoenixCluster;

use xai_candidate_pipeline::component_library::utils::current_timestamp_millis;
use xai_candidate_pipeline::scorer::Scorer;
use xai_recsys_proto::ProductSurface;

pub const PHOENIX_RANKER_KILL_SWITCH_DECIDER: &str = "disable_home_mixer_phoenix_ranker";

pub struct PhoenixScorer {
    pub dispatch: PredictionDispatch,
}

impl PhoenixScorer {
    fn resolve_cluster(query: &ScoredPostsQuery) -> PhoenixCluster {
        let configured_cluster =
            PhoenixCluster::parse(&query.params.get(PhoenixInferenceClusterId));

        let threshold: u64 = query.params.get(PhoenixRankerNewUserHistoryThreshold);
        if threshold > 0 {
            let action_count = query
                .scoring_sequence
                .as_ref()
                .and_then(|s| s.metadata.as_ref())
                .map(|m| m.length)
                .unwrap_or(0);

            if action_count < threshold {
                return PhoenixCluster::parse(
                    &query.params.get(PhoenixRankerNewUserInferenceClusterId),
                );
            }
        }

        if let Some(decider) = &query.decider {
            let is_prod = matches!(
                configured_cluster,
                PhoenixCluster::Experiment1Fou | PhoenixCluster::Experiment2Fou
            );
            if is_prod {
                if decider.enabled("override_qf_use_experiment2_fou") {
                    return PhoenixCluster::Experiment2Fou;
                }
                if decider.enabled("override_qf_use_experiment1_fou") {
                    return PhoenixCluster::Experiment1Fou;
                }
            }
        }

        configured_cluster
    }

    fn ranker_killed(query: &ScoredPostsQuery) -> bool {
        query
            .decider
            .as_ref()
            .is_some_and(|d| d.enabled(PHOENIX_RANKER_KILL_SWITCH_DECIDER))
    }

    /// Latest viewer-action time from the already-hydrated scoring sequence.
    /// `last_modified_epoch_ms` is when UAS last wrote; `last_sequence_time`
    /// is the newest action in the window. Either being newer than a frozen
    /// Phoenix head means For You is ranking on stale predictions.
    fn scoring_sequence_fresh_ms(query: &ScoredPostsQuery) -> Option<u64> {
        let meta = query.scoring_sequence.as_ref()?.metadata.as_ref()?;
        let ts = meta.last_modified_epoch_ms.max(meta.last_sequence_time);
        (ts > 0).then_some(ts)
    }

    fn cached_phoenix_scores_are_stale(query: &ScoredPostsQuery) -> bool {
        let Some(sequence_ms) = Self::scoring_sequence_fresh_ms(query) else {
            return false;
        };
        query.cached_posts.iter().any(|candidate| {
            candidate
                .last_scored_at_ms
                .is_none_or(|scored_at| scored_at < sequence_ms)
        })
    }

    /// Cache hits still hydrate a fresh UAS sequence. Skip Phoenix only when
    /// every cached head was scored at or after that sequence; otherwise the
    /// 180s Redis slate keeps VQV / dwell / fav predictions from before the
    /// viewer finished or skipped videos.
    fn should_score(query: &ScoredPostsQuery) -> bool {
        if Self::ranker_killed(query) {
            return false;
        }
        if !query.has_cached_posts {
            return true;
        }
        query.scoring_sequence.is_some() && Self::cached_phoenix_scores_are_stale(query)
    }
}

#[async_trait]
impl Scorer<ScoredPostsQuery, PostCandidate> for PhoenixScorer {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        Self::should_score(query)
    }

    async fn score(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let last_scored_at_ms = current_timestamp_millis();
        let product_surface = if query.in_network_only {
            ProductSurface::HomeTimelineRankedFollowing
        } else {
            ProductSurface::HomeTimelineRanking
        };

        if query.scoring_sequence.is_none() {
            return vec![Ok(PostCandidate::default()); candidates.len()];
        };

        let cluster = Self::resolve_cluster(query);
        let request = build_prediction_request(query, candidates, product_surface);

        let predictions = self
            .dispatch
            .predict_with_fallback(query, cluster, request)
            .await
            .map_err(|e| format!("Phoenix prediction failed: {}", e));

        let predictions = match predictions {
            Ok(predictions) => predictions,
            Err(err) => return vec![Err(err); candidates.len()],
        };

        candidates
            .iter()
            .map(|c| PostCandidate {
                phoenix_scores: predictions.candidate_scores(&c.get_original_tweet_id()),
                backbone_scores: predictions.candidate_backbone_scores(&c.get_original_tweet_id()),
                served_slate_context: predictions
                    .candidate_slate_context(&c.get_original_tweet_id())
                    .map(Into::into),
                prediction_request_id: Some(query.prediction_id),
                last_scored_at_ms,
                reranker_head_tag: Some(query.params.get(RerankerHeadTag) as u32),
                ..Default::default()
            })
            .map(Ok)
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, scored: PostCandidate) {
        candidate.phoenix_scores = scored.phoenix_scores;
        candidate.backbone_scores = scored.backbone_scores;
        candidate.served_slate_context = scored.served_slate_context;
        candidate.prediction_request_id = scored.prediction_request_id;
        candidate.last_scored_at_ms = scored.last_scored_at_ms;
        candidate.reranker_head_tag = scored.reranker_head_tag;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xai_recsys_proto::{UserActionSequence, UserActionSequenceMeta};

    fn sequence(last_modified_epoch_ms: u64, last_sequence_time: u64) -> UserActionSequence {
        UserActionSequence {
            metadata: Some(UserActionSequenceMeta {
                last_modified_epoch_ms,
                last_sequence_time,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn cached_query(
        last_scored_at_ms: Option<u64>,
        seq: Option<UserActionSequence>,
    ) -> ScoredPostsQuery {
        ScoredPostsQuery {
            has_cached_posts: true,
            cached_posts: vec![PostCandidate {
                tweet_id: 1,
                last_scored_at_ms,
                ..Default::default()
            }],
            scoring_sequence: seq,
            ..Default::default()
        }
    }

    #[test]
    fn scores_uncached_requests() {
        assert!(PhoenixScorer::should_score(&ScoredPostsQuery::default()));
    }

    #[test]
    fn keeps_cache_when_sequence_is_older_than_heads() {
        let query = cached_query(Some(2_000), Some(sequence(1_500, 1_400)));
        assert!(!PhoenixScorer::should_score(&query));
    }

    #[test]
    fn rescores_cache_when_sequence_is_newer_than_heads() {
        let query = cached_query(Some(1_000), Some(sequence(2_500, 2_400)));
        assert!(PhoenixScorer::should_score(&query));
    }

    #[test]
    fn rescores_cache_when_only_last_sequence_time_is_newer() {
        let query = cached_query(Some(1_000), Some(sequence(0, 1_001)));
        assert!(PhoenixScorer::should_score(&query));
    }

    #[test]
    fn rescores_cache_when_cached_head_has_no_score_time() {
        let query = cached_query(None, Some(sequence(2_000, 2_000)));
        assert!(PhoenixScorer::should_score(&query));
    }

    #[test]
    fn keeps_cache_when_sequence_has_no_usable_time() {
        let query = cached_query(Some(1_000), Some(sequence(0, 0)));
        assert!(!PhoenixScorer::should_score(&query));
    }

    #[test]
    fn keeps_cache_when_sequence_is_missing() {
        let query = cached_query(Some(1_000), None);
        assert!(!PhoenixScorer::should_score(&query));
    }

    #[test]
    fn rescores_when_any_cached_head_is_older_than_the_sequence() {
        let query = ScoredPostsQuery {
            has_cached_posts: true,
            cached_posts: vec![
                PostCandidate {
                    tweet_id: 1,
                    last_scored_at_ms: Some(3_000),
                    ..Default::default()
                },
                PostCandidate {
                    tweet_id: 2,
                    last_scored_at_ms: Some(1_000),
                    ..Default::default()
                },
            ],
            scoring_sequence: Some(sequence(2_000, 2_000)),
            ..Default::default()
        };
        assert!(PhoenixScorer::should_score(&query));
    }
}
