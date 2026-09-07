use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use xai_candidate_pipeline::filter::{Filter, FilterResult};
use xai_visibility_filtering::models::{Action, FilteredReason};

pub struct VFFilter;

impl Filter<ScoredPostsQuery, PostCandidate> for VFFilter {
    fn filter(
        &self,
        _query: &ScoredPostsQuery,
        candidates: Vec<PostCandidate>,
    ) -> FilterResult<PostCandidate> {
        let (removed, kept): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|c| should_drop(&c.visibility_reason));

        FilterResult { kept, removed }
    }
}

/// Home Mixer cannot render VF's soft hide actions. `Avoid` is Strato's
/// hide-from-timeline / suppress verdict (the sample
/// `homeMixerFilteredReason.Tweet` payload decodes to it). `Tombstone`
/// replaces the post. `NotEvaluated` is a missing action on an otherwise
/// present `SafetyResult` — fail closed, same as a None action.
///
/// `Interstitial` stays visible here: TimelineHome NSFW is an interstitial
/// for in-network posts. Ancillary wrappers drop it in
/// `should_drop_reason` because the quote/reply card has no warning UI.
fn safety_action_hides_from_timeline(action: &Action) -> bool {
    match action {
        Action::Allow | Action::Interstitial | Action::Downrank => false,
        Action::Drop(_) | Action::Avoid | Action::Tombstone | Action::NotEvaluated => true,
    }
}

fn should_drop(reason: &Option<FilteredReason>) -> bool {
    match reason {
        Some(FilteredReason::SafetyResult(safety_result)) => {
            safety_action_hides_from_timeline(&safety_result.action)
        }
        Some(_) => true,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate_with_reason(reason: Option<FilteredReason>) -> PostCandidate {
        PostCandidate {
            visibility_reason: reason,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn drops_when_action_is_drop() {
        let filter = VFFilter;
        let query = ScoredPostsQuery::default();

        let drop_reason =
            FilteredReason::SafetyResult(xai_visibility_filtering::models::SafetyResult {
                action: Action::Drop(Default::default()),
                ..Default::default()
            });
        let candidates = vec![
            candidate_with_reason(Some(drop_reason)),
            candidate_with_reason(None),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept.len(), 1);
    }

    #[tokio::test]
    async fn keeps_when_action_allows() {
        let filter = VFFilter;
        let query = ScoredPostsQuery::default();

        let allowed_reason =
            FilteredReason::SafetyResult(xai_visibility_filtering::models::SafetyResult {
                action: Action::Allow,
                ..Default::default()
            });
        let candidates = vec![candidate_with_reason(Some(allowed_reason))];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.removed.len(), 0);
        assert_eq!(result.kept.len(), 1);
    }

    #[tokio::test]
    async fn drops_when_user_filtered_state_present() {
        let filter = VFFilter;
        let query = ScoredPostsQuery::default();

        let reason = FilteredReason::AuthorBlockViewer;
        let candidates = vec![
            candidate_with_reason(Some(reason)),
            candidate_with_reason(None),
        ];

        let result = filter.filter(&query, candidates);

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.kept.len(), 1);
    }

    fn safety(action: Action) -> FilteredReason {
        FilteredReason::SafetyResult(xai_visibility_filtering::models::SafetyResult {
            action,
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn drops_avoid_tombstone_and_not_evaluated() {
        let filter = VFFilter;
        let query = ScoredPostsQuery::default();

        let result = filter.filter(
            &query,
            vec![
                candidate_with_reason(Some(safety(Action::Avoid))),
                candidate_with_reason(Some(safety(Action::Tombstone))),
                candidate_with_reason(Some(safety(Action::NotEvaluated))),
                candidate_with_reason(Some(safety(Action::Interstitial))),
                candidate_with_reason(Some(safety(Action::Downrank))),
                candidate_with_reason(None),
            ],
        );

        assert_eq!(result.removed.len(), 3);
        assert_eq!(result.kept.len(), 3);
        assert!(matches!(
            result.kept[0].visibility_reason,
            Some(FilteredReason::SafetyResult(ref sr)) if sr.action == Action::Interstitial
        ));
        assert!(matches!(
            result.kept[1].visibility_reason,
            Some(FilteredReason::SafetyResult(ref sr)) if sr.action == Action::Downrank
        ));
        assert_eq!(result.kept[2].visibility_reason, None);
    }
}
