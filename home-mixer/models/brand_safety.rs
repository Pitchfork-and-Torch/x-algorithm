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
}

pub(crate) const MEDIUM_RISK_LABELS: &[SafetyLabelType] = &[
    SafetyLabelType::NSFW_HIGH_PRECISION,
    SafetyLabelType::NSFW_HIGH_RECALL,
    SafetyLabelType::NSFA_HIGH_PRECISION,
    SafetyLabelType::NSFA_KEYWORDS_HIGH_PRECISION,
    SafetyLabelType::GORE_AND_VIOLENCE_HIGH_PRECISION,
    SafetyLabelType::NSFW_REPORTED_HEURISTICS,
    SafetyLabelType::GORE_AND_VIOLENCE_REPORTED_HEURISTICS,
    SafetyLabelType::NSFW_CARD_IMAGE,
    SafetyLabelType::DO_NOT_AMPLIFY,
    SafetyLabelType::MALICIOUS_URL,
    SafetyLabelType::NSFA_COMMUNITY_NOTE,
    SafetyLabelType::PDNA,
    SafetyLabelType::EGREGIOUS_NSFW,
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

// GetSafetyLabels returns expired rows. Drop rules already skip them.
// Ads adjacency still used type presence, so a lapsed Community Note
// (or any other TTL label) kept the post MediumRisk.
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

pub fn worst_verdict(a: &BrandSafetyVerdict, b: &BrandSafetyVerdict) -> BrandSafetyVerdict {
    if *a as i32 >= *b as i32 {
        *a
    } else {
        *b
    }
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
    fn medium_risk_with_nsfw_label() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA,
            SafetyLabelType::NSFW_HIGH_PRECISION,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
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
    fn medium_risk_trumps_low_risk() {
        let labels = labels_with(&[
            SafetyLabelType::GROK_SFA,
            SafetyLabelType::NSFA_LIMITED_INVENTORY,
            SafetyLabelType::NSFW_HIGH_PRECISION,
        ]);
        assert_eq!(
            compute_verdict(&labels, PRE_CUTOFF_ID),
            BrandSafetyVerdict::MediumRisk
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
            (SafetyLabelType::NSFW_HIGH_PRECISION, Some(2_000)),
        ]);
        assert_eq!(
            compute_verdict_at(&labels, PRE_CUTOFF_ID, 1_000),
            BrandSafetyVerdict::MediumRisk
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
}
