pub use xai_x_thrift::tweet_safety_label::SafetyLabelType;

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};
use xai_visibility_filtering_proto as vf_pb;

#[derive(Clone, Debug, Default)]
pub struct SafetyLabelMap(HashSet<SafetyLabelType>);

impl SafetyLabelMap {
    pub fn new(label_types: HashSet<SafetyLabelType>) -> Self {
        Self(label_types)
    }

    pub fn from_proto_label_types(proto: &vf_pb::SafetyLabelMap) -> Self {
        Self::from_proto_label_types_at(proto, now_msec())
    }

    // Drop rules only check type presence. Skip proto rows whose
    // expires_at_msec has passed; missing expiry stays permanent.
    fn from_proto_label_types_at(proto: &vf_pb::SafetyLabelMap, now_msec: i64) -> Self {
        Self(
            proto
                .labels
                .iter()
                .filter(|(_, label)| is_unexpired(label, now_msec))
                .map(|(&label_type, _)| SafetyLabelType(label_type))
                .collect(),
        )
    }

    #[inline]
    pub fn has_label(&self, label_type: SafetyLabelType) -> bool {
        self.0.contains(&label_type)
    }
}

fn now_msec() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn is_unexpired(label: &vf_pb::SafetyLabel, now_msec: i64) -> bool {
    match label.expires_at_msec {
        Some(exp) if exp <= now_msec => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn proto_with(expires_at_msec: Option<i64>) -> vf_pb::SafetyLabelMap {
        vf_pb::SafetyLabelMap {
            labels: HashMap::from([(
                i32::from(SafetyLabelType::SPAM_HIGH_RECALL),
                vf_pb::SafetyLabel {
                    expires_at_msec,
                    ..Default::default()
                },
            )]),
        }
    }

    #[test]
    fn missing_expiry_is_treated_as_permanent() {
        let map = SafetyLabelMap::from_proto_label_types_at(&proto_with(None), 1_000);
        assert!(map.has_label(SafetyLabelType::SPAM_HIGH_RECALL));
    }

    #[test]
    fn future_expiry_is_still_active() {
        let map = SafetyLabelMap::from_proto_label_types_at(&proto_with(Some(2_000)), 1_000);
        assert!(map.has_label(SafetyLabelType::SPAM_HIGH_RECALL));
    }

    #[test]
    fn past_expiry_is_not_treated_as_present() {
        let map = SafetyLabelMap::from_proto_label_types_at(&proto_with(Some(999)), 1_000);
        assert!(!map.has_label(SafetyLabelType::SPAM_HIGH_RECALL));
    }

    #[test]
    fn expiry_exactly_now_is_not_treated_as_present() {
        let map = SafetyLabelMap::from_proto_label_types_at(&proto_with(Some(1_000)), 1_000);
        assert!(!map.has_label(SafetyLabelType::SPAM_HIGH_RECALL));
    }

    #[test]
    fn expired_sibling_does_not_drop_an_active_label() {
        let proto = vf_pb::SafetyLabelMap {
            labels: HashMap::from([
                (
                    i32::from(SafetyLabelType::SPAM_HIGH_RECALL),
                    vf_pb::SafetyLabel {
                        expires_at_msec: Some(500),
                        ..Default::default()
                    },
                ),
                (
                    i32::from(SafetyLabelType::SPAM),
                    vf_pb::SafetyLabel {
                        expires_at_msec: Some(2_000),
                        ..Default::default()
                    },
                ),
            ]),
        };
        let map = SafetyLabelMap::from_proto_label_types_at(&proto, 1_000);
        assert!(!map.has_label(SafetyLabelType::SPAM_HIGH_RECALL));
        assert!(map.has_label(SafetyLabelType::SPAM));
    }
}
