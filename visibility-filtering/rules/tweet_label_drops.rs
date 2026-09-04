use crate::models::{SafetyLabelType, VfAction};
use crate::rules::{Rule, RuleContext};
use xai_visibility_filtering::models::{
    Action, DropReason, FilteredReason, SafetyResult, SafetyResultReason,
};

#[derive(Clone)]
pub struct SafetyLabelDropRule {
    name: &'static str,
    label: SafetyLabelType,
    reason: FilteredReason,
    exempt_author: bool,
}

impl SafetyLabelDropRule {
    pub const fn new(
        name: &'static str,
        label: SafetyLabelType,
        reason: FilteredReason,
        exempt_author: bool,
    ) -> Self {
        Self {
            name,
            label,
            reason,
            exempt_author,
        }
    }
}

impl Rule for SafetyLabelDropRule {
    fn name(&self) -> &'static str {
        self.name
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        if !context.has_tweet_safety_label(self.label) {
            return VfAction::Allow;
        }
        if self.exempt_author && context.is_author_viewer() {
            return VfAction::Allow;
        }
        VfAction::Drop(self.reason.clone())
    }
}

const NSFW_HIGH_PRECISION_REASON: FilteredReason = FilteredReason::SafetyResult(SafetyResult {
    reason: Some(SafetyResultReason::NsfwHighPrecision),
    action: Action::Drop(DropReason {}),
});

pub const PDNA_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "PdnaTweetLabelRule",
    SafetyLabelType::PDNA,
    NSFW_HIGH_PRECISION_REASON,
    true,
);
pub const BOUNCE_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "BounceTweetLabelRule",
    SafetyLabelType::BOUNCE,
    FilteredReason::TweetIsBounced,
    true,
);
pub const SPAM_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "SpamTweetLabelRule",
    SafetyLabelType::SPAM,
    FilteredReason::PossiblyUndesirable,
    true,
);
pub const SPAM_HIGH_RECALL_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "SpamHighRecallDropRule",
    SafetyLabelType::SPAM_HIGH_RECALL,
    FilteredReason::PossiblyUndesirable,
    true,
);
pub const NSFW_TEXT_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "NsfwTextTweetLabelDropRule",
    SafetyLabelType::NSFW_TEXT,
    NSFW_HIGH_PRECISION_REASON,
    true,
);
pub const NSFW_HIGH_RECALL_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "NsfwHighRecallDropRule",
    SafetyLabelType::NSFW_HIGH_RECALL,
    FilteredReason::ContainNsfwMedia,
    true,
);
pub const FOR_EMERGENCY_USE_ONLY_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "ForEmergencyUseOnlyDropRule",
    SafetyLabelType::FOR_EMERGENCY_USE_ONLY,
    FilteredReason::UnspecifiedReason,
    false,
);

pub const NSFW_HIGH_PRECISION_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "NsfwHighPrecisionOonDropRule",
    SafetyLabelType::NSFW_HIGH_PRECISION,
    FilteredReason::ContainNsfwMedia,
    true,
);
pub const NSFW_CARD_IMAGE_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "NsfwCardImageOonDropRule",
    SafetyLabelType::NSFW_CARD_IMAGE,
    FilteredReason::ContainNsfwMedia,
    true,
);
pub const GORE_AND_VIOLENCE_HIGH_PRECISION_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "GoreAndViolenceOonDropRule",
    SafetyLabelType::GORE_AND_VIOLENCE_HIGH_PRECISION,
    FilteredReason::ContainNsfwMedia,
    true,
);
pub const DO_NOT_AMPLIFY_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "DoNotAmplifyOonDropRule",
    SafetyLabelType::DO_NOT_AMPLIFY,
    FilteredReason::PossiblyUndesirable,
    true,
);
pub const MALICIOUS_URL_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "MaliciousUrlOonDropRule",
    SafetyLabelType::MALICIOUS_URL,
    FilteredReason::PossiblyUndesirable,
    true,
);

pub const FOSNR_HATEFUL_CONDUCT_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "FosnrHatefulConductDropRule",
    SafetyLabelType::FOSNR_HATEFUL_CONDUCT,
    FilteredReason::PossiblyUndesirable,
    true,
);
pub const FOSNR_VIOLENT_SPEECH_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "FosnrViolentSpeechDropRule",
    SafetyLabelType::FOSNR_VIOLENT_SPEECH,
    FilteredReason::PossiblyUndesirable,
    true,
);
pub const FOSNR_ABUSE_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "FosnrAbuseDropRule",
    SafetyLabelType::FOSNR_ABUSE,
    FilteredReason::PossiblyUndesirable,
    true,
);
pub const FOSNR_CIVIC_INTEGRITY_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "FosnrCivicIntegrityDropRule",
    SafetyLabelType::FOSNR_CIVIC_INTEGRITY,
    FilteredReason::PossiblyUndesirable,
    true,
);

pub const FOSNR_ABUSE_INSULTS_OON_DROP: SafetyLabelDropRule = SafetyLabelDropRule::new(
    "FosnrAbuseInsultsOonDropRule",
    SafetyLabelType::FOSNR_ABUSE_INSULTS,
    FilteredReason::PossiblyUndesirable,
    true,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        HydratedTweetCandidate, SafetyLabel, SafetyLabelMap, Viewer, ViewerFeatures,
    };
    use std::collections::HashMap;

    fn candidate_with_label(label: SafetyLabelType) -> HydratedTweetCandidate {
        let mut labels = HashMap::new();
        labels.insert(label, SafetyLabel::default());
        HydratedTweetCandidate {
            tweet_id: 1,
            author_id: 100,
            safety_labels: SafetyLabelMap::new(labels),
            ..Default::default()
        }
    }

    fn viewer() -> ViewerFeatures {
        ViewerFeatures {
            viewer: Viewer::LoggedIn(999),
            ..Default::default()
        }
    }

    fn author_viewer() -> ViewerFeatures {
        ViewerFeatures {
            viewer: Viewer::LoggedIn(100),
            ..Default::default()
        }
    }

    #[test]
    fn drops_non_author_with_mapped_reason() {
        let c = candidate_with_label(SafetyLabelType::PDNA);
        assert!(matches!(
            PDNA_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(FilteredReason::SafetyResult(_))
        ));

        let c = candidate_with_label(SafetyLabelType::BOUNCE);
        assert!(matches!(
            BOUNCE_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(FilteredReason::TweetIsBounced)
        ));

        let c = candidate_with_label(SafetyLabelType::SPAM);
        assert!(matches!(
            SPAM_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(FilteredReason::PossiblyUndesirable)
        ));

        let c = candidate_with_label(SafetyLabelType::NSFW_HIGH_RECALL);
        assert!(matches!(
            NSFW_HIGH_RECALL_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(FilteredReason::ContainNsfwMedia)
        ));
    }

    #[test]
    fn author_exempt_rules_allow_self_view() {
        let c = candidate_with_label(SafetyLabelType::PDNA);
        assert!(matches!(
            PDNA_DROP.evaluate(&crate::rules::test_context(&author_viewer(), &c)),
            VfAction::Allow
        ));
        let c = candidate_with_label(SafetyLabelType::DO_NOT_AMPLIFY);
        assert!(matches!(
            DO_NOT_AMPLIFY_DROP.evaluate(&crate::rules::test_context(&author_viewer(), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn all_viewer_rules_drop_even_for_author() {
        let c = candidate_with_label(SafetyLabelType::FOR_EMERGENCY_USE_ONLY);
        assert!(matches!(
            FOR_EMERGENCY_USE_ONLY_DROP.evaluate(&crate::rules::test_context(&author_viewer(), &c)),
            VfAction::Drop(FilteredReason::UnspecifiedReason)
        ));
    }

    #[test]
    fn oon_media_drops_regardless_of_opt_in() {
        let opted_in = ViewerFeatures {
            viewer: Viewer::LoggedIn(999),
            allows_sensitive_media: true,
            ..Default::default()
        };
        let c = candidate_with_label(SafetyLabelType::NSFW_HIGH_PRECISION);
        assert!(matches!(
            NSFW_HIGH_PRECISION_DROP.evaluate(&crate::rules::test_context(&opted_in, &c)),
            VfAction::Drop(FilteredReason::ContainNsfwMedia)
        ));
    }

    #[test]
    fn no_label_allows() {
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            ..Default::default()
        };
        assert!(matches!(
            PDNA_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn expired_proto_label_allows() {
        let proto = xai_visibility_filtering_proto::SafetyLabelMap {
            labels: HashMap::from([(
                i32::from(SafetyLabelType::SPAM_HIGH_RECALL),
                xai_visibility_filtering_proto::SafetyLabel {
                    expires_at_msec: Some(1),
                    ..Default::default()
                },
            )]),
        };
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            author_id: 100,
            safety_labels: SafetyLabelMap::from_proto_label_types(&proto),
            ..Default::default()
        };
        assert!(matches!(
            SPAM_HIGH_RECALL_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    fn follower_viewer() -> ViewerFeatures {
        ViewerFeatures {
            viewer: Viewer::LoggedIn(999),
            ..Default::default()
        }
    }

    fn candidate_with_label_followed(label: SafetyLabelType) -> HydratedTweetCandidate {
        let mut c = candidate_with_label(label);
        c.relationship.viewer_follows_author = true;
        c
    }

    #[test]
    fn fosnr_level3_drops_non_author_including_follower() {
        for rule in [
            &FOSNR_HATEFUL_CONDUCT_DROP,
            &FOSNR_VIOLENT_SPEECH_DROP,
            &FOSNR_ABUSE_DROP,
            &FOSNR_CIVIC_INTEGRITY_DROP,
        ] {
            let c = candidate_with_label(rule.label);
            assert!(matches!(
                rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
                VfAction::Drop(FilteredReason::PossiblyUndesirable)
            ));
            let c = candidate_with_label_followed(rule.label);
            assert!(matches!(
                rule.evaluate(&crate::rules::test_context(&follower_viewer(), &c)),
                VfAction::Drop(FilteredReason::PossiblyUndesirable)
            ));
        }
    }

    #[test]
    fn fosnr_level3_exempts_author() {
        for rule in [
            &FOSNR_HATEFUL_CONDUCT_DROP,
            &FOSNR_VIOLENT_SPEECH_DROP,
            &FOSNR_ABUSE_DROP,
            &FOSNR_CIVIC_INTEGRITY_DROP,
        ] {
            let c = candidate_with_label(rule.label);
            assert!(matches!(
                rule.evaluate(&crate::rules::test_context(&author_viewer(), &c)),
                VfAction::Allow
            ));
        }
    }

    #[test]
    fn malicious_url_drops_non_author_and_exempts_author() {
        let c = candidate_with_label(SafetyLabelType::MALICIOUS_URL);
        assert!(matches!(
            MALICIOUS_URL_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(FilteredReason::PossiblyUndesirable)
        ));
        assert!(matches!(
            MALICIOUS_URL_DROP.evaluate(&crate::rules::test_context(&author_viewer(), &c)),
            VfAction::Allow
        ));
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            ..Default::default()
        };
        assert!(matches!(
            MALICIOUS_URL_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn fosnr_abuse_insults_oon_drops_all_non_authors() {
        let c = candidate_with_label(SafetyLabelType::FOSNR_ABUSE_INSULTS);
        assert!(matches!(
            FOSNR_ABUSE_INSULTS_OON_DROP.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(FilteredReason::PossiblyUndesirable)
        ));
        let c = candidate_with_label_followed(SafetyLabelType::FOSNR_ABUSE_INSULTS);
        assert!(matches!(
            FOSNR_ABUSE_INSULTS_OON_DROP
                .evaluate(&crate::rules::test_context(&follower_viewer(), &c)),
            VfAction::Drop(FilteredReason::PossiblyUndesirable)
        ));
        let c = candidate_with_label(SafetyLabelType::FOSNR_ABUSE_INSULTS);
        assert!(matches!(
            FOSNR_ABUSE_INSULTS_OON_DROP
                .evaluate(&crate::rules::test_context(&author_viewer(), &c)),
            VfAction::Allow
        ));
    }
}
