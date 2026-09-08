use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use xai_x_thrift::tweet_safety_label::{SafetyLabel, SafetyLabelSource, SafetyLabelType};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(i32)]
pub enum BrandSafetyVerdict {
    #[default]
    Unspecified = 0,
    Safe = 1,
    LowRisk = 2,
    MediumRisk = 3,
    HighRisk = 4,
}

pub(crate) const HIGH_RISK_LABELS: &[SafetyLabelType] = &[
    SafetyLabelType::NSFW_HIGH_PRECISION,
    SafetyLabelType::NSFW_HIGH_RECALL,
    SafetyLabelType::GORE_AND_VIOLENCE_HIGH_PRECISION,
    SafetyLabelType::PDNA,
    SafetyLabelType::EGREGIOUS_NSFW,
    SafetyLabelType::SOFT_NSFW,
];

pub(crate) const MEDIUM_RISK_LABELS: &[SafetyLabelType] = &[
    SafetyLabelType::NSFA_HIGH_PRECISION,
    SafetyLabelType::NSFA_KEYWORDS_HIGH_PRECISION,
    SafetyLabelType::NSFW_REPORTED_HEURISTICS,
    SafetyLabelType::GORE_AND_VIOLENCE_REPORTED_HEURISTICS,
    SafetyLabelType::NSFW_CARD_IMAGE,
    SafetyLabelType::DO_NOT_AMPLIFY,
    SafetyLabelType::MALICIOUS_URL,
    SafetyLabelType::NSFA_COMMUNITY_NOTE,
    SafetyLabelType::GROK_NSFA,
    SafetyLabelType::NSFW_TEXT,
];

pub(crate) const LOW_RISK_LABELS: &[SafetyLabelType] = &[
    SafetyLabelType::NSFA_LIMITED_INVENTORY,
    SafetyLabelType::GROK_NSFA_LIMITED,
    SafetyLabelType::NSFA_HIGH_RECALL,
];

const PTOS_CUTOFF_TWEET_ID: u64 = 2_054_275_414_225_846_272;

fn now_msec() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// GetSafetyLabels returns expired rows. VF drop rules already skip them.
// Ads adjacency still used type presence, so a lapsed Community Note
// (or any other TTL label) kept the post MediumRisk / HighRisk.
fn is_unexpired(label: &SafetyLabel, now_msec: i64) -> bool {
    match label.expires_at_msec {
        Some(exp) if exp <= now_msec => false,
        _ => true,
    }
}

fn has_active(
    labels: &HashMap<SafetyLabelType, SafetyLabel>,
    label_type: SafetyLabelType,
    now_msec: i64,
) -> bool {
    labels
        .get(&label_type)
        .is_some_and(|label| is_unexpired(label, now_msec))
}

pub(crate) fn is_active_label(label: &SafetyLabel) -> bool {
    is_unexpired(label, now_msec())
}

pub fn compute_verdict(
    labels: &HashMap<SafetyLabelType, SafetyLabel>,
    tweet_id: u64,
) -> BrandSafetyVerdict {
    compute_verdict_at(labels, tweet_id, now_msec())
}

fn compute_verdict_at(
    labels: &HashMap<SafetyLabelType, SafetyLabel>,
    tweet_id: u64,
    now_msec: i64,
) -> BrandSafetyVerdict {
    if HIGH_RISK_LABELS
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return BrandSafetyVerdict::HighRisk;
    }

    if MEDIUM_RISK_LABELS
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return BrandSafetyVerdict::MediumRisk;
    }

    let scored_by_grok = has_active(labels, SafetyLabelType::GROK_SFA, now_msec)
        || has_active(labels, SafetyLabelType::GROK_NSFA_LIMITED, now_msec);
    if !scored_by_grok {
        return BrandSafetyVerdict::MediumRisk;
    }

    if tweet_id >= PTOS_CUTOFF_TWEET_ID
        && !has_active(labels, SafetyLabelType::PTOS_REVIEWED, now_msec)
    {
        return BrandSafetyVerdict::MediumRisk;
    }

    if LOW_RISK_LABELS
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return BrandSafetyVerdict::LowRisk;
    }

    BrandSafetyVerdict::Safe
}

pub(crate) const MEDIUM_RISK_LABELS_V2: &[SafetyLabelType] = &[
    SafetyLabelType::NSFA_KEYWORDS_HIGH_PRECISION,
    SafetyLabelType::NSFW_REPORTED_HEURISTICS,
    SafetyLabelType::GORE_AND_VIOLENCE_REPORTED_HEURISTICS,
    SafetyLabelType::NSFW_CARD_IMAGE,
    SafetyLabelType::DO_NOT_AMPLIFY,
    SafetyLabelType::MALICIOUS_URL,
    SafetyLabelType::NSFA_COMMUNITY_NOTE,
    SafetyLabelType::GROK_NSFA_V2,
    SafetyLabelType::GROK_NSFA_EXPANDED_V2,
    SafetyLabelType::NSFW_TEXT,
];

pub(crate) const LOW_RISK_LABELS_V2: &[SafetyLabelType] = &[
    SafetyLabelType::GROK_NSFA_LIMITED_V2,
    SafetyLabelType::NSFA_HIGH_RECALL,
];

const V2_WRITTEN_LABELS: &[SafetyLabelType] = &[
    SafetyLabelType::GROK_SFA_V2,
    SafetyLabelType::GROK_NSFA_V2,
    SafetyLabelType::GROK_NSFA_LIMITED_V2,
    SafetyLabelType::GROK_NSFA_EXPANDED_V2,
];

pub(crate) fn compute_verdict_v2(
    labels: &HashMap<SafetyLabelType, SafetyLabel>,
    tweet_id: u64,
) -> BrandSafetyVerdict {
    compute_verdict_v2_at(labels, tweet_id, now_msec())
}

fn compute_verdict_v2_at(
    labels: &HashMap<SafetyLabelType, SafetyLabel>,
    tweet_id: u64,
    now_msec: i64,
) -> BrandSafetyVerdict {
    if !V2_WRITTEN_LABELS
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return compute_verdict_at(labels, tweet_id, now_msec);
    }
    if HIGH_RISK_LABELS
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return BrandSafetyVerdict::HighRisk;
    }
    if MEDIUM_RISK_LABELS_V2
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return BrandSafetyVerdict::MediumRisk;
    }

    let scored_by_grok = has_active(labels, SafetyLabelType::GROK_SFA_V2, now_msec)
        || has_active(labels, SafetyLabelType::GROK_NSFA_LIMITED_V2, now_msec);
    if !scored_by_grok {
        return BrandSafetyVerdict::MediumRisk;
    }

    if tweet_id >= PTOS_CUTOFF_TWEET_ID
        && !has_active(labels, SafetyLabelType::PTOS_REVIEWED, now_msec)
    {
        return BrandSafetyVerdict::MediumRisk;
    }

    if LOW_RISK_LABELS_V2
        .iter()
        .any(|l| has_active(labels, *l, now_msec))
    {
        return BrandSafetyVerdict::LowRisk;
    }

    BrandSafetyVerdict::Safe
}

pub fn worst_verdict(a: &BrandSafetyVerdict, b: &BrandSafetyVerdict) -> BrandSafetyVerdict {
    if *a as i32 >= *b as i32 {
        *a
    } else {
        *b
    }
}

/// Verdict written onto a scored organic post when ads brand-safety hydration
/// did not set one. Unspecified is not "avoid" in the For You ads blender, so
/// a missed ads-VF read cannot demote the organic by score. Following still
/// refuses to sit an ad next to Unspecified.
pub fn scored_post_verdict(verdict: Option<BrandSafetyVerdict>) -> BrandSafetyVerdict {
    verdict.unwrap_or(BrandSafetyVerdict::Unspecified)
}

pub(crate) fn botmaker_rule_id_from(label: &SafetyLabel) -> Option<i64> {
    label.safety_label_source.as_ref().and_then(|src| {
        if let SafetyLabelSource::BotMakerAction(action) = src {
            Some(action.rule_id)
        } else {
            None
        }
    })
}

pub(crate) fn botmaker_rule_category(rule_id: i64) -> &'static str {
    match rule_id {
        1000..=1099 => "Content",
        1100..=1199 => "ContentLimited",
        1200..=1399 => "Safety",
        1400..=1499 => "Grok",
        1500..=1600 => "Quote",
        _ => "Legacy",
    }
}

pub(crate) fn truncate_description(s: &str) -> String {
    s.chars().take(250).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels_with(types: &[SafetyLabelType]) -> HashMap<SafetyLabelType, SafetyLabel> {
        types.iter().map(|t| (*t, SafetyLabel::default())).collect()
    }

    const POST_CUTOFF_ID: u64 = PTOS_CUTOFF_TWEET_ID;

    const PRE_CUTOFF_ID: u64 = PTOS_CUTOFF_TWEET_ID - 1;

    #[test]
    fn safe_with_grok_sfa_only() {
        let labels = labels_with(&[SafetyLabelType::GROK_SFA]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn high_risk_with_nsfw_label() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA,
            SafetyLabelType::NSFW_HIGH_PRECISION,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::HighRisk
        );
    }

    #[test]
    fn medium_risk_with_grok_nsfa() {
        let labels = labels_with(&[SafetyLabelType::GROK_SFA, SafetyLabelType::GROK_NSFA]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn low_risk_with_limited_inventory() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA,
            SafetyLabelType::NSFA_LIMITED_INVENTORY,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::LowRisk
        );
    }

    #[test]
    fn low_risk_with_grok_nsfa_limited() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA,
            SafetyLabelType::GROK_NSFA_LIMITED,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::LowRisk
        );
    }

    #[test]
    fn low_risk_with_grok_nsfa_limited_without_grok_sfa() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_NSFA_LIMITED,
            SafetyLabelType::NSFA_LIMITED_INVENTORY,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::LowRisk
        );
    }

    #[test]
    fn high_risk_trumps_low_risk() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA,
            SafetyLabelType::NSFA_LIMITED_INVENTORY,
            SafetyLabelType::NSFW_HIGH_PRECISION,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::HighRisk
        );
    }

    #[test]
    fn post_cutoff_medium_risk_without_ptos_reviewed() {
        let labels = labels_with(&[SafetyLabelType::GROK_SFA]);
        assert_eq!(
            compute_verdict(&labels, POST_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn post_cutoff_safe_with_grok_sfa_and_ptos_reviewed() {
        let labels = labels_with(&[SafetyLabelType::GROK_SFA, SafetyLabelType::PTOS_REVIEWED]);
        assert_eq!(
            compute_verdict(&labels, POST_CUTOFF_ID),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn pre_cutoff_safe_with_grok_sfa_only() {
        let labels = labels_with(&[SafetyLabelType::GROK_SFA]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn v2_defers_to_v1_when_v2_has_not_ruled() {
        let v1_safe = labels_with(&[SafetyLabelType::GROK_SFA]);
        assert_eq!(
            compute_verdict_v2(&v1_safe, PRE_CUTOFF_ID),
            compute_verdict(&v1_safe, PRE_CUTOFF_ID)
        );
        assert_eq!(
            compute_verdict_v2(&v1_safe, PRE_CUTOFF_ID),
            BrandSafetyVerdict::Safe
        );

        let v1_nsfa = labels_with(&[SafetyLabelType::GROK_NSFA]);
        assert_eq!(
            compute_verdict_v2(&v1_nsfa, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );
        let v1_limited = labels_with(&[
            SafetyLabelType::NSFA_LIMITED_INVENTORY,
            SafetyLabelType::GROK_NSFA_LIMITED,
        ]);
        assert_eq!(
            compute_verdict_v2(&v1_limited, PRE_CUTOFF_ID),
            BrandSafetyVerdict::LowRisk
        );

        assert_eq!(
            compute_verdict_v2(&labels_with(&[]), PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );

        let disagreement = labels_with(&[SafetyLabelType::GROK_SFA, SafetyLabelType::GROK_NSFA_V2]);
        assert_eq!(
            compute_verdict_v2(&disagreement, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );

        let freed = labels_with(&[
            SafetyLabelType::NSFA_HIGH_PRECISION,
            SafetyLabelType::GROK_NSFA,
            SafetyLabelType::GROK_SFA_V2,
        ]);
        assert_eq!(
            compute_verdict(&freed, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );
        assert_eq!(
            compute_verdict_v2(&freed, PRE_CUTOFF_ID),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn v2_mirrors_v1_across_tier_matrix() {
        fn to_v2(v1_set: &[SafetyLabelType]) -> Vec<SafetyLabelType> {
            v1_set
                .iter()
                .filter_map(|l| match *l {
                    SafetyLabelType::GROK_SFA => Some(SafetyLabelType::GROK_SFA_V2),
                    SafetyLabelType::GROK_NSFA => Some(SafetyLabelType::GROK_NSFA_V2),
                    SafetyLabelType::GROK_NSFA_LIMITED => {
                        Some(SafetyLabelType::GROK_NSFA_LIMITED_V2)
                    }
                    SafetyLabelType::NSFA_HIGH_PRECISION
                    | SafetyLabelType::NSFA_LIMITED_INVENTORY => None,
                    other => Some(other),
                })
                .collect()
        }

        let matrix: &[&[SafetyLabelType]] = &[
            &[],
            &[SafetyLabelType::GROK_SFA],
            &[SafetyLabelType::GROK_SFA, SafetyLabelType::PTOS_REVIEWED],
            &[
                SafetyLabelType::NSFA_LIMITED_INVENTORY,
                SafetyLabelType::GROK_NSFA_LIMITED,
            ],
            &[
                SafetyLabelType::NSFA_LIMITED_INVENTORY,
                SafetyLabelType::GROK_NSFA_LIMITED,
                SafetyLabelType::PTOS_REVIEWED,
            ],
            &[
                SafetyLabelType::GROK_SFA,
                SafetyLabelType::NSFA_HIGH_PRECISION,
                SafetyLabelType::GROK_NSFA,
            ],
        ];

        for v1_set in matrix {
            let v2_set = to_v2(v1_set);
            for tweet_id in [PRE_CUTOFF_ID, POST_CUTOFF_ID] {
                assert_eq!(
                    compute_verdict(&labels_with(v1_set), tweet_id),
                    compute_verdict_v2(&labels_with(&v2_set), tweet_id),
                    "v1 {v1_set:?} vs v2 {v2_set:?} at tweet_id {tweet_id}"
                );
            }
        }

        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA_V2,
            SafetyLabelType::GROK_NSFA_EXPANDED_V2,
        ]);
        assert_eq!(
            compute_verdict_v2(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
        );

        use std::collections::HashSet;
        let medium: HashSet<_> = MEDIUM_RISK_LABELS.iter().copied().collect();
        let medium_v2: HashSet<_> = MEDIUM_RISK_LABELS_V2.iter().copied().collect();
        let expected_medium_v2: HashSet<_> = medium
            .iter()
            .copied()
            .filter(|l| {
                *l != SafetyLabelType::GROK_NSFA && *l != SafetyLabelType::NSFA_HIGH_PRECISION
            })
            .chain([
                SafetyLabelType::GROK_NSFA_V2,
                SafetyLabelType::GROK_NSFA_EXPANDED_V2,
            ])
            .collect();
        assert_eq!(medium_v2, expected_medium_v2);

        let low: HashSet<_> = LOW_RISK_LABELS.iter().copied().collect();
        let low_v2: HashSet<_> = LOW_RISK_LABELS_V2.iter().copied().collect();
        let expected_low_v2: HashSet<_> = low
            .iter()
            .copied()
            .filter(|l| {
                *l != SafetyLabelType::GROK_NSFA_LIMITED
                    && *l != SafetyLabelType::NSFA_LIMITED_INVENTORY
            })
            .chain([SafetyLabelType::GROK_NSFA_LIMITED_V2])
            .collect();
        assert_eq!(low_v2, expected_low_v2);
    }

    #[test]
    fn worst_verdict_ordering() {
        assert_eq!(
            worst_verdict(&BrandSafetyVerdict::Safe, &BrandSafetyVerdict::LowRisk),
            BrandSafetyVerdict::LowRisk
        );
        assert_eq!(
            worst_verdict(
                &BrandSafetyVerdict::LowRisk,
                &BrandSafetyVerdict::MediumRisk
            ),
            BrandSafetyVerdict::MediumRisk
        );
        assert_eq!(
            worst_verdict(&BrandSafetyVerdict::MediumRisk, &BrandSafetyVerdict::Safe),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn missing_ads_vf_verdict_is_unspecified_not_medium_risk() {
        assert_eq!(scored_post_verdict(None), BrandSafetyVerdict::Unspecified);
        assert_eq!(
            scored_post_verdict(Some(BrandSafetyVerdict::Safe)),
            BrandSafetyVerdict::Safe
        );
        assert_eq!(
            scored_post_verdict(Some(BrandSafetyVerdict::MediumRisk)),
            BrandSafetyVerdict::MediumRisk
        );
        assert_eq!(
            scored_post_verdict(Some(BrandSafetyVerdict::HighRisk)),
            BrandSafetyVerdict::HighRisk
        );
    }

    fn label_with_expiry(expires_at_msec: Option<i64>) -> SafetyLabel {
        SafetyLabel {
            expires_at_msec,
            ..Default::default()
        }
    }

    fn labels_with_expiry(
        types: &[(SafetyLabelType, Option<i64>)],
    ) -> HashMap<SafetyLabelType, SafetyLabel> {
        types
            .iter()
            .map(|(t, exp)| (*t, label_with_expiry(*exp)))
            .collect()
    }

    #[test]
    fn expired_community_note_does_not_keep_medium_risk() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, Some(999)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn future_community_note_is_still_medium_risk() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, Some(2_000)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn missing_expiry_on_community_note_is_treated_as_permanent() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, None),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn expiry_exactly_now_is_not_treated_as_active() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, Some(1_000)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn expired_sibling_does_not_hide_an_active_medium_risk_label() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, Some(500)),
            (SafetyLabelType::NSFW_TEXT, Some(2_000)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn expired_high_risk_label_does_not_keep_high_risk() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFW_HIGH_PRECISION, Some(999)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn expired_low_risk_label_does_not_keep_low_risk() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::NSFA_LIMITED_INVENTORY, Some(999)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn expired_grok_sfa_is_treated_as_unscored() {
        let labels = labels_with_expiry(&[(SafetyLabelType::GROK_SFA, Some(999))]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::MediumRisk
        );
    }

    #[test]
    fn v2_expired_community_note_does_not_keep_medium_risk() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA_V2, None),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, Some(999)),
        ]);
        assert_eq!(
            compute_verdict_v2_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::Safe
        );
    }

    #[test]
    fn v2_expired_written_labels_fall_back_to_v1() {
        let labels = labels_with_expiry(&[
            (SafetyLabelType::GROK_SFA, None),
            (SafetyLabelType::GROK_SFA_V2, Some(999)),
            (SafetyLabelType::NSFA_COMMUNITY_NOTE, None),
        ]);
        assert_eq!(
            compute_verdict_v2_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::MediumRisk
        );
    }
}
