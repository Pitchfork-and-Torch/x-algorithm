use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::scorers::ranking_scorer::{RankingScorer, ScoringWeights};
use tonic::async_trait;
use xai_candidate_pipeline::scorer::Scorer;

pub struct PhoenixScoresRankingScorer;

#[async_trait]
impl Scorer<ScoredPostsQuery, PostCandidate> for PhoenixScoresRankingScorer {
    fn enable(&self, _query: &ScoredPostsQuery) -> bool {
        true
    }

    async fn score(
        &self,
        query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let weights = ScoringWeights::from_params(&query.params);
        candidates
            .iter()
            .map(|c| {
                if !RankingScorer::phoenix_heads_present(&c.phoenix_scores) {
                    return Ok(PostCandidate {
                        weighted_score: None,
                        score: None,
                        ..Default::default()
                    });
                }
                let weighted = RankingScorer::compute_weighted_score(&weights, query, c);
                Ok(PostCandidate {
                    weighted_score: Some(weighted),
                    score: Some(weighted),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, scored: PostCandidate) {
        candidate.weighted_score = scored.weighted_score;
        candidate.score = scored.score;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::candidate::PhoenixScores;

    fn query() -> ScoredPostsQuery {
        let mut query = ScoredPostsQuery::default();
        let fs = xai_feature_switches::FeatureSwitches::new(vec![]).unwrap();
        let results = fs.match_recipient(&xai_feature_switches::RecipientBuilder::new().build());
        query.params = results.into();
        query
    }

    #[tokio::test]
    async fn missing_heads_leave_score_unset() {
        let scored = PhoenixScoresRankingScorer
            .score(&query(), &[PostCandidate::default()])
            .await;
        let out = scored[0].as_ref().unwrap();
        assert!(out.score.is_none());
        assert!(out.weighted_score.is_none());
    }

    #[tokio::test]
    async fn zero_favorite_head_still_ranks() {
        let candidate = PostCandidate {
            phoenix_scores: PhoenixScores {
                favorite_score: Some(0.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let scored = PhoenixScoresRankingScorer
            .score(&query(), std::slice::from_ref(&candidate))
            .await;
        let out = scored[0].as_ref().unwrap();
        assert!(out.score.is_some());
        assert!(out.weighted_score.is_some());
    }
}
