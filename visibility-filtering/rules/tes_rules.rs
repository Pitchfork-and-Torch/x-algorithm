use crate::models::VfAction;
use crate::rules::{Rule, RuleContext};
use xai_core_entities::entities::TakedownReason;
use xai_visibility_filtering::models::FilteredReason;

pub struct DropStaleTweetsRule;

impl Rule for DropStaleTweetsRule {
    fn name(&self) -> &'static str {
        "DropStaleTweetsRule"
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        let candidate = context.candidate();
        if !candidate.is_stale_tweet() {
            return VfAction::Allow;
        }
        if candidate.is_retweet() {
            return VfAction::Allow;
        }
        if candidate.tweet_features.core.source_tweet_id.is_some() {
            return VfAction::Allow;
        }
        VfAction::Drop(FilteredReason::UnspecifiedReason)
    }
}

pub struct DropLegalTakendownPostRule;

impl Rule for DropLegalTakendownPostRule {
    fn name(&self) -> &'static str {
        "DropLegalTakendownPostRule"
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        if viewer_in_withheld_country(context, legal_takedown_country) {
            return VfAction::Drop(FilteredReason::UnspecifiedReason);
        }
        VfAction::Allow
    }
}

pub struct DropLocalLawsTakendownPostRule;

impl Rule for DropLocalLawsTakendownPostRule {
    fn name(&self) -> &'static str {
        "DropLocalLawsTakendownPostRule"
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        if viewer_in_withheld_country(context, local_laws_takedown_country) {
            return VfAction::Drop(FilteredReason::UnspecifiedReason);
        }
        VfAction::Allow
    }
}

fn legal_takedown_country(reason: &TakedownReason) -> Option<&str> {
    match reason {
        TakedownReason::LegalRequest { country_code }
        | TakedownReason::UnspecifiedReason { country_code } => Some(country_code),
        _ => None,
    }
}

fn local_laws_takedown_country(reason: &TakedownReason) -> Option<&str> {
    match reason {
        TakedownReason::BystanderReport { country_code } => Some(country_code),
        _ => None,
    }
}

fn viewer_in_withheld_country(
    context: &RuleContext<'_>,
    extractor: impl Fn(&TakedownReason) -> Option<&str>,
) -> bool {
    let viewer = context.viewer();
    let candidate = context.candidate();
    if context.is_author_viewer() {
        return false;
    }
    let Some(viewer_country) = &viewer.country_code else {
        return false;
    };
    candidate
        .tweet_features
        .takedown
        .reasons
        .iter()
        .filter_map(extractor)
        .any(|c| c.eq_ignore_ascii_case(viewer_country))
}

fn is_global_takedown_reason(reason: &TakedownReason) -> bool {
    matches!(
        reason,
        TakedownReason::Dmca | TakedownReason::HatefulImagery | TakedownReason::Unknown
    )
}

fn has_country_takedown_reason(reason: &TakedownReason) -> bool {
    legal_takedown_country(reason).is_some() || local_laws_takedown_country(reason).is_some()
}

/// Post-level DMCA / hateful-imagery / unknown takedowns, or TES `has_takedown`
/// with no country-scoped reason left for the geo rules.
pub struct DropGlobalTakendownPostRule;

impl Rule for DropGlobalTakendownPostRule {
    fn name(&self) -> &'static str {
        "DropGlobalTakendownPostRule"
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        let takedown = &context.candidate().tweet_features.takedown;
        let has_global_reason = takedown.reasons.iter().any(is_global_takedown_reason);
        let has_country_reason = takedown.reasons.iter().any(has_country_takedown_reason);
        if has_global_reason || (takedown.applied && !has_country_reason) {
            return VfAction::Drop(FilteredReason::UnspecifiedReason);
        }
        VfAction::Allow
    }
}

pub struct DropTweetsWithGeoRestrictedMediaRule;

const WORLDWIDE_COUNTRY_CODE: &str = "xx";

impl Rule for DropTweetsWithGeoRestrictedMediaRule {
    fn name(&self) -> &'static str {
        "DropTweetsWithGeoRestrictedMediaRule"
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        let viewer = context.viewer();
        let candidate = context.candidate();
        let country = viewer
            .country_code
            .as_deref()
            .unwrap_or(WORLDWIDE_COUNTRY_CODE);
        let allow = &candidate.tweet_features.media.geo_allow_list;
        let deny = &candidate.tweet_features.media.geo_deny_list;
        if (!allow.is_empty() && !allow.iter().any(|c| c.eq_ignore_ascii_case(country)))
            || deny.iter().any(|c| c.eq_ignore_ascii_case(country))
        {
            return VfAction::Drop(FilteredReason::UnspecifiedReason);
        }
        VfAction::Allow
    }
}

pub struct DropTweetsWithDmcaMediaRule;

impl Rule for DropTweetsWithDmcaMediaRule {
    fn name(&self) -> &'static str {
        "DropTweetsWithDmcaMediaRule"
    }

    fn evaluate(&self, context: &RuleContext<'_>) -> VfAction {
        let candidate = context.candidate();
        if candidate.has_dmca_media() {
            return VfAction::Drop(FilteredReason::UnspecifiedReason);
        }
        VfAction::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CoreFeature, HydratedTweetCandidate, MediaFeature, TakedownFeature, TweetFeatures, Viewer,
        ViewerFeatures,
    };
    use xai_core_entities::entities::{EditControl, EditControlInitial};

    fn viewer() -> ViewerFeatures {
        ViewerFeatures {
            viewer: Viewer::LoggedIn(999),
            ..Default::default()
        }
    }

    fn stale_edit_control() -> Option<EditControl> {
        Some(EditControl::Initial(EditControlInitial {
            edit_tweet_ids: vec![1, 2],
            ..Default::default()
        }))
    }

    #[test]
    fn stale_edit_drops() {
        let rule = DropStaleTweetsRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                edit_control: stale_edit_control(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn non_stale_tweet_allows() {
        let rule = DropStaleTweetsRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn stale_retweet_allows() {
        let rule = DropStaleTweetsRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                edit_control: stale_edit_control(),
                core: CoreFeature {
                    source_tweet_id: Some(99),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    fn viewer_with_country(country: &str) -> ViewerFeatures {
        ViewerFeatures {
            viewer: Viewer::LoggedIn(999),
            country_code: Some(country.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn takedown_drops_in_matching_country() {
        let rule = DropLegalTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![
                        TakedownReason::LegalRequest {
                            country_code: "de".to_string(),
                        },
                        TakedownReason::UnspecifiedReason {
                            country_code: "fr".to_string(),
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn takedown_allows_in_non_matching_country() {
        let rule = DropLegalTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::LegalRequest {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("us"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn takedown_allows_when_no_viewer_country() {
        let rule = DropLegalTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::LegalRequest {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn legal_rule_ignores_local_laws_countries() {
        let rule = DropLegalTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::BystanderReport {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn local_laws_drops_in_matching_country() {
        let rule = DropLocalLawsTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![
                        TakedownReason::BystanderReport {
                            country_code: "de".to_string(),
                        },
                        TakedownReason::BystanderReport {
                            country_code: "fr".to_string(),
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("fr"), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn local_laws_allows_in_non_matching_country() {
        let rule = DropLocalLawsTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::BystanderReport {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("us"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn local_laws_ignores_legal_countries() {
        let rule = DropLocalLawsTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::LegalRequest {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn legal_allows_author_viewing_own_withheld_post() {
        let rule = DropLegalTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            author_id: 999,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::LegalRequest {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn local_laws_allows_author_viewing_own_withheld_post() {
        let rule = DropLocalLawsTakendownPostRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            author_id: 999,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![TakedownReason::BystanderReport {
                        country_code: "de".to_string(),
                    }],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn takedown_rules_ignore_non_country_reasons() {
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                takedown: TakedownFeature {
                    reasons: vec![
                        TakedownReason::Dmca,
                        TakedownReason::HatefulImagery,
                        TakedownReason::Unknown,
                    ],
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            DropLegalTakendownPostRule
                .evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Allow
        ));
        assert!(matches!(
            DropLocalLawsTakendownPostRule
                .evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Allow
        ));
        assert!(matches!(
            DropGlobalTakendownPostRule
                .evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Drop(_)
        ));
    }

    fn takedown_candidate(takedown: TakedownFeature) -> HydratedTweetCandidate {
        HydratedTweetCandidate {
            tweet_id: 1,
            author_id: 100,
            tweet_features: TweetFeatures {
                takedown,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn global_takedown_drops_dmca() {
        let c = takedown_candidate(TakedownFeature {
            reasons: vec![TakedownReason::Dmca],
            ..Default::default()
        });
        assert!(matches!(
            DropGlobalTakendownPostRule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn global_takedown_drops_hateful_imagery() {
        let c = takedown_candidate(TakedownFeature {
            reasons: vec![TakedownReason::HatefulImagery],
            ..Default::default()
        });
        assert!(matches!(
            DropGlobalTakendownPostRule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn global_takedown_drops_unknown() {
        let c = takedown_candidate(TakedownFeature {
            reasons: vec![TakedownReason::Unknown],
            ..Default::default()
        });
        assert!(matches!(
            DropGlobalTakendownPostRule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn global_takedown_drops_applied_with_no_country_reason() {
        let c = takedown_candidate(TakedownFeature {
            applied: true,
            reasons: vec![],
        });
        assert!(matches!(
            DropGlobalTakendownPostRule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn global_takedown_allows_country_only_legal_request() {
        let c = takedown_candidate(TakedownFeature {
            applied: true,
            reasons: vec![TakedownReason::LegalRequest {
                country_code: "de".to_string(),
            }],
        });
        assert!(matches!(
            DropGlobalTakendownPostRule
                .evaluate(&crate::rules::test_context(&viewer_with_country("us"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn global_takedown_drops_even_for_author() {
        let mut c = takedown_candidate(TakedownFeature {
            reasons: vec![TakedownReason::Dmca],
            ..Default::default()
        });
        c.author_id = 999;
        assert!(matches!(
            DropGlobalTakendownPostRule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn global_takedown_does_not_consume_media_dmca() {
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                media: MediaFeature {
                    has_dmca_media: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            DropGlobalTakendownPostRule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    fn geo_candidate(allow: &[&str], deny: &[&str]) -> HydratedTweetCandidate {
        HydratedTweetCandidate {
            tweet_id: 1,
            author_id: 100,
            tweet_features: TweetFeatures {
                media: MediaFeature {
                    geo_allow_list: allow.iter().map(|s| s.to_string()).collect(),
                    geo_deny_list: deny.iter().map(|s| s.to_string()).collect(),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn geo_restricted_no_restrictions_allows() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&[], &[]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("us"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn geo_restricted_denylist_drops_matching_country() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&[], &["de", "fr"]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn geo_restricted_denylist_allows_other_country() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&[], &["de", "fr"]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("us"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn geo_restricted_allowlist_allows_listed_country() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&["us", "gb"], &[]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("us"), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn geo_restricted_allowlist_drops_unlisted_country() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&["us", "gb"], &[]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn geo_restricted_country_matching_is_case_insensitive() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(
                &viewer_with_country("us"),
                &geo_candidate(&["US"], &[])
            )),
            VfAction::Allow
        ));
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(
                &viewer_with_country("de"),
                &geo_candidate(&[], &["DE"])
            )),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn geo_restricted_missing_country_fails_nonempty_allowlist() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&["us"], &[]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn geo_restricted_missing_country_matches_xx_denylist() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&[], &["xx"]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn geo_restricted_missing_country_not_in_denylist_allows() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let c = geo_candidate(&[], &["de"]);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Allow
        ));
    }

    #[test]
    fn geo_restricted_drops_even_for_author() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let mut c = geo_candidate(&[], &["de"]);
        c.author_id = 999;
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn geo_restricted_drops_retweets_too() {
        let rule = DropTweetsWithGeoRestrictedMediaRule;
        let mut c = geo_candidate(&[], &["de"]);
        c.tweet_features.core.source_tweet_id = Some(99);
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer_with_country("de"), &c)),
            VfAction::Drop(_)
        ));
    }

    #[test]
    fn dmca_drops() {
        let rule = DropTweetsWithDmcaMediaRule;
        let c = HydratedTweetCandidate {
            tweet_id: 1,
            tweet_features: TweetFeatures {
                media: MediaFeature {
                    has_dmca_media: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            rule.evaluate(&crate::rules::test_context(&viewer(), &c)),
            VfAction::Drop(_)
        ));
    }
}
