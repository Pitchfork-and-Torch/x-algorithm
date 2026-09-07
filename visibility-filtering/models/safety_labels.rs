pub use xai_x_thrift::tweet_safety_label::SafetyLabelType;

use std::collections::HashSet;
use xai_visibility_filtering_proto as vf_pb;

#[derive(Clone, Debug, Default)]
pub struct SafetyLabelMap(HashSet<SafetyLabelType>);

impl SafetyLabelMap {
    #[cfg(test)]
    pub fn new(label_types: HashSet<SafetyLabelType>) -> Self {
        Self(label_types)
    }

    pub fn from_proto_label_types(proto: &vf_pb::SafetyLabelMap) -> Self {
        Self(
            proto
                .labels
                .keys()
                .map(|label_type| SafetyLabelType(*label_type))
                .collect(),
        )
    }

    #[inline]
    pub fn has_label(&self, label_type: SafetyLabelType) -> bool {
        self.0.contains(&label_type)
    }

    pub fn union(&mut self, other: &Self) {
        self.0.extend(other.0.iter().copied());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_adds_civic_without_dropping_existing() {
        let mut labels = SafetyLabelMap::new(HashSet::from([SafetyLabelType::SPAM]));
        labels.union(&SafetyLabelMap::new(HashSet::from([
            SafetyLabelType::FOSNR_CIVIC_INTEGRITY,
        ])));
        assert!(labels.has_label(SafetyLabelType::SPAM));
        assert!(labels.has_label(SafetyLabelType::FOSNR_CIVIC_INTEGRITY));
    }
}
