use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use crate::params::{AuthorSizeIpsAlpha, AuthorSizeIpsMaxBoost, EnableAuthorSizeIps};

/// Horvitz-Thompson residual of author audience size.
///
/// Phoenix already estimates P(action | viewer, post). Follower count is not a
/// Phoenix feature, but size still leaks into the slate via the follow graph,
/// hashed author IDs, SimClusters log-fav retrieval, and the flat OON tax.
/// This multiplier residualizes ln(1 + followers) inside the scored batch so
/// two posts with the same Phoenix score are not ranked by author size.
///
///   p_i   = ln(1 + max(followers_i, 1))
///   ips_i = 1 / p_i
///   m_i   = 1 + alpha * (ips_i / mean(ips) - 1)
///   m_i   = clamp(m_i, 1 / max_boost, max_boost)
///
/// Mean-normalization keeps the batch arithmetic mean at 1: this reallocates
/// score, it does not inflate it. Quality still wins: a 3x Phoenix gap beats
/// the default 2x clamp. Missing follower counts are identity (multiplier 1).
/// Negative scores are not boosted.
///
/// This is individual meritocratic fairness (Singh and Joachims 2018, groups
/// of size one; Biega et al. equity of attention). It is not a demographic
/// quota.
pub(crate) fn multipliers_for(query: &ScoredPostsQuery, candidates: &[PostCandidate]) -> Vec<f64> {
    if !query.params.get(EnableAuthorSizeIps) {
        return vec![1.0; candidates.len()];
    }
    multipliers(
        candidates.iter().map(|c| c.author_followers_count),
        query.params.get(AuthorSizeIpsAlpha),
        query.params.get(AuthorSizeIpsMaxBoost),
    )
}

pub(crate) fn apply(
    query: &ScoredPostsQuery,
    candidates: &[PostCandidate],
    scores: &[f64],
) -> Vec<f64> {
    let multipliers = multipliers_for(query, candidates);
    scores
        .iter()
        .zip(multipliers)
        .map(|(&score, multiplier)| {
            if score > 0.0 && multiplier.is_finite() {
                score * multiplier
            } else {
                score
            }
        })
        .collect()
}

pub(crate) fn multipliers(
    followers: impl IntoIterator<Item = Option<i32>>,
    alpha: f64,
    max_boost: f64,
) -> Vec<f64> {
    let followers: Vec<Option<i32>> = followers.into_iter().collect();
    let n = followers.len();
    if n == 0 || !alpha.is_finite() || alpha <= 0.0 {
        return vec![1.0; n];
    }

    let cap = if max_boost.is_finite() && max_boost >= 1.0 {
        max_boost
    } else {
        1.0
    };
    let floor = 1.0 / cap;

    let mut ips = vec![None; n];
    let mut ips_sum = 0.0;
    let mut known = 0usize;
    for (i, follower_count) in followers.iter().enumerate() {
        let Some(raw) = follower_count else {
            continue;
        };
        let propensity = (1.0 + f64::from((*raw).max(1))).ln();
        if !propensity.is_finite() || propensity <= 0.0 {
            continue;
        }
        let weight = 1.0 / propensity;
        if !weight.is_finite() || weight <= 0.0 {
            continue;
        }
        ips[i] = Some(weight);
        ips_sum += weight;
        known += 1;
    }

    if known == 0 {
        return vec![1.0; n];
    }

    let mean_ips = ips_sum / known as f64;
    if !mean_ips.is_finite() || mean_ips <= 0.0 {
        return vec![1.0; n];
    }

    ips.into_iter()
        .map(|weight| match weight {
            Some(weight) => {
                let raw = 1.0 + alpha * (weight / mean_ips - 1.0);
                if raw.is_finite() {
                    raw.clamp(floor, cap)
                } else {
                    1.0
                }
            }
            None => 1.0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn identity_when_alpha_is_zero() {
        let m = multipliers([Some(100), Some(1_000_000)], 0.0, 2.0);
        assert_eq!(m, vec![1.0, 1.0]);
    }

    #[test]
    fn identity_when_followers_missing() {
        let m = multipliers([None, None], 0.5, 2.0);
        assert_eq!(m, vec![1.0, 1.0]);
    }

    #[test]
    fn identity_when_all_authors_same_size() {
        let m = multipliers([Some(12_000), Some(12_000), Some(12_000)], 0.5, 2.0);
        for value in m {
            approx(value, 1.0);
        }
    }

    #[test]
    fn missing_followers_stay_identity_among_sized_authors() {
        let m = multipliers([Some(100), None, Some(1_000_000)], 0.5, 2.0);
        assert!(m[0] > 1.0, "small author should lift: {}", m[0]);
        approx(m[1], 1.0);
        assert!(m[2] < 1.0, "large author should recede: {}", m[2]);
    }

    #[test]
    fn smaller_author_gets_higher_multiplier() {
        let m = multipliers([Some(100), Some(10_000), Some(1_000_000)], 0.5, 2.0);
        assert!(m[0] > m[1] && m[1] > m[2], "{m:?}");
        assert!(m[0] > 1.0 && m[2] < 1.0, "{m:?}");
    }

    #[test]
    fn mean_of_known_multipliers_is_one() {
        let m = multipliers([Some(50), Some(5_000), Some(500_000)], 0.5, 8.0);
        let mean = m.iter().sum::<f64>() / m.len() as f64;
        approx(mean, 1.0);
    }

    #[test]
    fn clamp_respects_max_boost() {
        let m = multipliers([Some(1), Some(2_000_000_000)], 8.0, 1.5);
        assert!(m[0] <= 1.5 + 1e-12, "{}", m[0]);
        assert!(m[1] >= 1.0 / 1.5 - 1e-12, "{}", m[1]);
    }

    #[test]
    fn zero_followers_treated_as_one() {
        let a = multipliers([Some(0), Some(10_000)], 0.5, 2.0);
        let b = multipliers([Some(1), Some(10_000)], 0.5, 2.0);
        approx(a[0], b[0]);
        approx(a[1], b[1]);
    }

    #[test]
    fn apply_does_not_boost_non_positive_scores() {
        let mut query = ScoredPostsQuery::default();
        let fs = xai_feature_switches::FeatureSwitches::new(vec![]).unwrap();
        let mut results =
            fs.match_recipient(&xai_feature_switches::RecipientBuilder::new().build());
        results.override_fs("rust_home_mixer_enable_author_size_ips".into(), "true");
        results.override_fs("rust_home_mixer_author_size_ips_alpha".into(), "1.0");
        query.params = results.into();

        let candidates = vec![
            PostCandidate {
                author_followers_count: Some(10),
                ..Default::default()
            },
            PostCandidate {
                author_followers_count: Some(1_000_000),
                ..Default::default()
            },
        ];
        let out = apply(&query, &candidates, &[-2.0, 0.0]);
        assert_eq!(out, vec![-2.0, 0.0]);
    }

    #[test]
    fn apply_lifts_small_author_on_positive_score() {
        let mut query = ScoredPostsQuery::default();
        let fs = xai_feature_switches::FeatureSwitches::new(vec![]).unwrap();
        let mut results =
            fs.match_recipient(&xai_feature_switches::RecipientBuilder::new().build());
        results.override_fs("rust_home_mixer_enable_author_size_ips".into(), "true");
        results.override_fs("rust_home_mixer_author_size_ips_alpha".into(), "0.5");
        results.override_fs("rust_home_mixer_author_size_ips_max_boost".into(), "2.0");
        query.params = results.into();

        let candidates = vec![
            PostCandidate {
                author_followers_count: Some(100),
                ..Default::default()
            },
            PostCandidate {
                author_followers_count: Some(1_000_000),
                ..Default::default()
            },
        ];
        let out = apply(&query, &candidates, &[10.0, 10.0]);
        assert!(out[0] > out[1], "{out:?}");
        assert!(out[0] > 10.0 && out[1] < 10.0, "{out:?}");
    }

    #[test]
    fn equal_phoenix_gap_still_loses_to_large_quality() {
        // Default clamp is 2x. A 3x Phoenix advantage must still win.
        let m = multipliers([Some(100), Some(1_000_000)], 0.5, 2.0);
        let small = 1.0 * m[0];
        let large = 3.0 * m[1];
        assert!(
            large > small,
            "quality must still dominate: {small} vs {large}"
        );
    }
}
