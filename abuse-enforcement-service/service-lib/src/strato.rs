use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use backon::Retryable;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{info, warn};
use xai_strato::Strato;

use crate::allowlist::{AllowlistEntry, ManhattanAllowlist};
use crate::entities::{self, HighPageRankUser};
use crate::facts::{AllowlistFacts, CredFacts, EntityType, GizmoduckFacts};
use crate::gizmoduck::GizmoduckCoreClient;
use crate::service::{record_strato_metrics, strato_retry_strategy};

#[derive(Clone)]
pub struct CredClients {
    pub high_page_rank: Arc<Strato>,
    pub grey_badge: Arc<Strato>,
}

async fn strato_fetch<K, V, T>(
    strato: &Strato,
    key: K,
    view: Option<V>,
    metric_label: &str,
) -> Result<Option<T>>
where
    K: Serialize + Clone + Send + Sync + 'static,
    V: Serialize + Clone + Send + Sync + 'static,
    T: DeserializeOwned,
{
    let start = std::time::Instant::now();
    let mut retries: u32 = 0;
    let result = (|| async { strato.fetch(key.clone(), view.clone()).await })
        .retry(strato_retry_strategy())
        .notify(|err, dur| {
            retries += 1;
            warn!("{metric_label} fetch failed (retrying in {dur:?}): {err}");
        })
        .await;
    match result {
        Ok(resp) => {
            let parsed: entities::StratoResponse<T> = serde_json::from_value(resp)
                .map_err(|e| anyhow::anyhow!("{metric_label} parse error: {e}"))?;
            record_strato_metrics(metric_label, "ok", start.elapsed(), retries);
            Ok(parsed.v)
        }
        Err(e) => {
            record_strato_metrics(metric_label, "error", start.elapsed(), retries);
            Err(anyhow::anyhow!(
                "{metric_label} fetch failed after retries: {e}"
            ))
        }
    }
}

/// Turns the result of an allowlist store lookup into facts.
///
/// A store error is propagated rather than mapped to `is_allowlisted: false`:
/// the allowlist is an exemption, so "could not check" must stop enforcement
/// (the caller retries) instead of silently reading as "not exempt". Only a
/// confirmed `Ok(None)` from the store means the entity is not allowlisted.
fn allowlist_facts(lookup: Result<Option<(AllowlistEntry, i64)>>) -> Result<AllowlistFacts> {
    Ok(match lookup? {
        Some((entry, ttl_secs)) => AllowlistFacts {
            is_allowlisted: true,
            added_by: Some(entry.added_by),
            reason: Some(entry.reason),
            ttl_secs: Some(ttl_secs),
        },
        None => AllowlistFacts::default(),
    })
}

#[tracing::instrument(skip_all, fields(is_allowlisted))]
pub async fn fetch_user_allowlist(
    allowlist: Option<&ManhattanAllowlist>,
    user_id: i64,
) -> Result<AllowlistFacts> {
    let span = tracing::Span::current();
    let Some(al) = allowlist else {
        span.record("is_allowlisted", false);
        return Ok(AllowlistFacts::default());
    };
    let facts = allowlist_facts(al.get_entity(EntityType::User, user_id).await)?;
    span.record("is_allowlisted", facts.is_allowlisted);
    if facts.is_allowlisted {
        warn!(
            added_by = facts.added_by.as_deref(),
            ttl_secs = facts.ttl_secs,
            reason = facts.reason.as_deref(),
            "user is in allowlist; will skip",
        );
    }
    Ok(facts)
}

#[tracing::instrument(skip_all, fields(entity_type = entity_type.as_str(), is_allowlisted))]
pub async fn fetch_entity_allowlist(
    allowlist: Option<&ManhattanAllowlist>,
    entity_type: EntityType,
    entity_id: i64,
) -> Result<AllowlistFacts> {
    let span = tracing::Span::current();
    let Some(al) = allowlist else {
        span.record("is_allowlisted", false);
        return Ok(AllowlistFacts::default());
    };
    let facts = allowlist_facts(al.get_entity(entity_type, entity_id).await)?;
    span.record("is_allowlisted", facts.is_allowlisted);
    if facts.is_allowlisted {
        warn!(
            entity_id,
            added_by = facts.added_by.as_deref(),
            ttl_secs = facts.ttl_secs,
            reason = facts.reason.as_deref(),
            "entity is in allowlist; will skip",
        );
    }
    Ok(facts)
}

pub async fn fetch_user(gd: &GizmoduckCoreClient, user_id: i64) -> Result<GizmoduckFacts> {
    gd.fetch_user(user_id).await
}

pub fn compose_cred(hpr: HighPageRankUser, is_grey_badge: bool) -> CredFacts {
    let is_high_page_rank = hpr.is_high_page_rank_user.unwrap_or(false);
    CredFacts {
        is_high: Some(is_grey_badge || is_high_page_rank),
        verified_type: None,
        score: hpr.user_cred_score,
        follower_count: hpr.follower_count,
    }
}

async fn fetch_hpr_column(strato: &Strato, user_id: i64) -> Result<HighPageRankUser> {
    Ok(strato_fetch(strato, user_id, None::<()>, "high_page_rank")
        .await?
        .unwrap_or_default())
}

async fn fetch_grey_column(strato: &Strato, user_id: i64) -> Result<bool> {
    Ok(
        strato_fetch::<_, (), bool>(strato, user_id, None::<()>, "grey_badge")
            .await?
            .unwrap_or(false),
    )
}

#[tracing::instrument(skip(clients), fields(is_high, verified_type, score, follower_count))]
pub async fn fetch_cred(clients: &CredClients, user_id: i64) -> Result<CredFacts> {
    let (hpr_res, grey_res) = tokio::join!(
        fetch_hpr_column(&clients.high_page_rank, user_id),
        fetch_grey_column(&clients.grey_badge, user_id),
    );
    let facts = compose_cred(hpr_res?, grey_res?);

    let span = tracing::Span::current();
    span.record("is_high", tracing::field::debug(&facts.is_high));
    span.record("verified_type", tracing::field::debug(&facts.verified_type));
    span.record("score", tracing::field::debug(&facts.score));
    span.record(
        "follower_count",
        tracing::field::debug(&facts.follower_count),
    );

    info!(
        ?facts.is_high,
        ?facts.verified_type,
        follower_count = ?facts.follower_count,
        score = ?facts.score,
        "cred fetch ok",
    );

    Ok(facts)
}

#[tracing::instrument(skip(uas_strato), fields(result))]
pub async fn fetch_uas(uas_strato: &Strato, user_id: i64) -> Option<String> {
    let resp: Result<Option<serde_json::Value>> =
        strato_fetch(uas_strato, user_id, None::<()>, "uas").await;
    let span = tracing::Span::current();
    match resp {
        Ok(Some(v)) => {
            span.record("result", "ok");
            info!("UAS response: {v}");
            Some(v.to_string())
        }
        Ok(None) => {
            span.record("result", "empty");
            info!("UAS returned null");
            None
        }
        Err(e) => {
            span.record("result", "error");
            warn!("UAS fetch failed: {e}");
            None
        }
    }
}

#[allow(dead_code)]
pub fn empty_info_map() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::HighPageRankUser;

    #[test]
    fn compose_cred_grey_only() {
        let hpr = HighPageRankUser {
            is_high_page_rank_user: Some(false),
            user_cred_score: Some(12.0),
            follower_count: Some(100),
        };
        let f = compose_cred(hpr, true);
        assert_eq!(f.is_high, Some(true));
        assert_eq!(f.score, Some(12.0));
        assert_eq!(f.follower_count, Some(100));
        assert_eq!(f.verified_type, None);
    }

    #[test]
    fn compose_cred_hpr_only() {
        let hpr = HighPageRankUser {
            is_high_page_rank_user: Some(true),
            user_cred_score: Some(60.0),
            follower_count: Some(50_000),
        };
        let f = compose_cred(hpr, false);
        assert_eq!(f.is_high, Some(true));
        assert_eq!(f.score, Some(60.0));
        assert_eq!(f.follower_count, Some(50_000));
    }

    #[test]
    fn compose_cred_neither() {
        let hpr = HighPageRankUser {
            is_high_page_rank_user: Some(false),
            user_cred_score: None,
            follower_count: Some(10),
        };
        let f = compose_cred(hpr, false);
        assert_eq!(f.is_high, Some(false));
        assert_eq!(f.score, None);
        assert_eq!(f.follower_count, Some(10));
    }

    #[test]
    fn compose_cred_missing_hpr_flag_defaults_false() {
        let hpr = HighPageRankUser::default();
        let f = compose_cred(hpr, false);
        assert_eq!(f.is_high, Some(false));
    }

    #[test]
    fn high_page_rank_user_serde_camel_case() {
        let json = r#"{
            "isHighPageRankUser": true,
            "userCredScore": 61.5,
            "followerCount": "42000"
        }"#;
        let u: HighPageRankUser = serde_json::from_str(json).unwrap();
        assert_eq!(u.is_high_page_rank_user, Some(true));
        assert_eq!(u.user_cred_score, Some(61.5));
        assert_eq!(u.follower_count, Some(42_000));
    }

    #[test]
    fn grey_badge_envelope_serde() {
        let json = r#"{"v": true}"#;
        let r: entities::StratoResponse<bool> = serde_json::from_str(json).unwrap();
        assert_eq!(r.v, Some(true));
    }

    #[test]
    fn allowlist_facts_absent_is_not_allowlisted() {
        let f = allowlist_facts(Ok(None)).unwrap();
        assert!(!f.is_allowlisted);
        assert_eq!(f.added_by, None);
        assert_eq!(f.reason, None);
        assert_eq!(f.ttl_secs, None);
    }

    #[test]
    fn allowlist_facts_present_is_allowlisted() {
        let entry = AllowlistEntry {
            added_by: "oncall".into(),
            reason: "false positive".into(),
            added_at: 1_700_000_000,
        };
        let f = allowlist_facts(Ok(Some((entry, 3600)))).unwrap();
        assert!(f.is_allowlisted);
        assert_eq!(f.added_by.as_deref(), Some("oncall"));
        assert_eq!(f.reason.as_deref(), Some("false positive"));
        assert_eq!(f.ttl_secs, Some(3600));
    }

    #[test]
    fn allowlist_facts_store_error_is_not_read_as_not_allowlisted() {
        let err = allowlist_facts(Err(anyhow::anyhow!("manhattan unavailable")));
        assert!(
            err.is_err(),
            "a failed allowlist lookup must propagate as an error, not as is_allowlisted=false"
        );
        assert!(err
            .unwrap_err()
            .to_string()
            .contains("manhattan unavailable"));
    }
}
