use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::params;
use xai_candidate_pipeline::selector::Selector;

pub struct TopKScoreSelector;

impl Selector<ScoredPostsQuery, PostCandidate> for TopKScoreSelector {
    fn score(&self, candidate: &PostCandidate) -> f64 {
        candidate
            .score
            .filter(|s| s.is_finite())
            .unwrap_or(f64::NEG_INFINITY)
    }
    fn size(&self) -> Option<usize> {
        Some(params::TOP_K_CANDIDATES_TO_SELECT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::candidate::PostCandidate;
    use xai_candidate_pipeline::selector::Selector;

    #[test]
    fn non_finite_scores_sort_last() {
        let selector = TopKScoreSelector;
        assert_eq!(
            selector.score(&PostCandidate {
                score: Some(1.5),
                ..Default::default()
            }),
            1.5
        );
        assert_eq!(
            selector.score(&PostCandidate {
                score: None,
                ..Default::default()
            }),
            f64::NEG_INFINITY
        );
        assert_eq!(
            selector.score(&PostCandidate {
                score: Some(f64::NAN),
                ..Default::default()
            }),
            f64::NEG_INFINITY
        );
        assert_eq!(
            selector.score(&PostCandidate {
                score: Some(f64::INFINITY),
                ..Default::default()
            }),
            f64::NEG_INFINITY
        );
    }
}
