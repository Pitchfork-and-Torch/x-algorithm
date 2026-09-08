use crate::clients::gizmoduck_client::GizmoduckClient;
use crate::models::candidate::PostCandidate;
use crate::models::query::ScoredPostsQuery;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::utils::{default_quick_cache, QuickCache};
use xai_candidate_pipeline::hydrator::{CacheStore, CachedHydrator};
use xai_core_entities::entities::GizmoduckUserResult;
use xai_x_thrift::user_labels::LabelValue;

pub struct GizmoduckCandidateHydrator {
    pub gizmoduck_client: Arc<dyn GizmoduckClient + Send + Sync>,
    pub cache: QuickCache<GizmoduckCacheKey, GizmoduckCacheValue>,
}

impl GizmoduckCandidateHydrator {
    pub async fn new(gizmoduck_client: Arc<dyn GizmoduckClient + Send + Sync>) -> Self {
        let cache = default_quick_cache();
        Self {
            gizmoduck_client,
            cache,
        }
    }
}

/// Store outcome for one requested account.
///
/// `Unknown` is a miss or a read error. Hydrator `Err` is dropped by
/// `update_all`, so unknown trust must be written as fail-closed values on
/// an `Ok` candidate or the post keeps empty NSFW/size fields and is scored
/// as clean / size-neutral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthorLookup {
    Found {
        followers_count: Option<i32>,
        screen_name: Option<String>,
        nsfw: bool,
        nsfw_ads: bool,
    },
    ConfirmedMissing,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuthorHydration {
    pub author_followers_count: Option<i32>,
    pub author_screen_name: Option<String>,
    pub retweeted_screen_name: Option<String>,
    pub nsfw_author: Option<bool>,
    pub nsfw_author_ads: Option<bool>,
    pub nsfw_author_phoenix: Option<bool>,
}

fn flag_from_lookup(lookup: &AuthorLookup, ads: bool) -> Option<bool> {
    match lookup {
        AuthorLookup::Found { nsfw, nsfw_ads, .. } => Some(if ads { *nsfw_ads } else { *nsfw }),
        AuthorLookup::ConfirmedMissing => Some(false),
        AuthorLookup::Unknown => None,
    }
}

fn or_trust(
    poster: &AuthorLookup,
    original: Option<&AuthorLookup>,
    quoted: Option<&AuthorLookup>,
    ads: bool,
) -> Option<bool> {
    let mut known = false;
    for lookup in [Some(poster), original, quoted].iter().copied().flatten() {
        match flag_from_lookup(lookup, ads) {
            None => return Some(true),
            Some(true) => return Some(true),
            Some(false) => known = true,
        }
    }
    known.then_some(false)
}

/// Size and NSFW for one candidate.
///
/// Follower count is the content origin (`retweeted_user_id` when present).
/// A small account amplifying a large original must not inherit the small
/// account's size residual. Quotes stay on the quoter: a quote is new copy.
///
/// NSFW is the OR of poster, original author, and quoted author. A store miss
/// or read error on any of those ids is labeled, not treated as clean.
pub(crate) fn hydrate_author_features(
    poster: AuthorLookup,
    original: Option<AuthorLookup>,
    quoted: Option<AuthorLookup>,
) -> AuthorHydration {
    let size_account = original.as_ref().unwrap_or(&poster);
    let author_followers_count = match size_account {
        AuthorLookup::Found {
            followers_count, ..
        } => *followers_count,
        AuthorLookup::ConfirmedMissing | AuthorLookup::Unknown => None,
    };
    let author_screen_name = match &poster {
        AuthorLookup::Found { screen_name, .. } => screen_name.clone(),
        AuthorLookup::ConfirmedMissing | AuthorLookup::Unknown => None,
    };
    let retweeted_screen_name = match &original {
        Some(AuthorLookup::Found { screen_name, .. }) => screen_name.clone(),
        _ => None,
    };
    let nsfw_author = or_trust(&poster, original.as_ref(), quoted.as_ref(), false);
    let nsfw_broad = or_trust(&poster, original.as_ref(), quoted.as_ref(), true);

    AuthorHydration {
        author_followers_count,
        author_screen_name,
        retweeted_screen_name,
        nsfw_author,
        nsfw_author_ads: nsfw_broad,
        nsfw_author_phoenix: nsfw_broad,
    }
}

fn user_ids_to_fetch(candidates: &[PostCandidate]) -> Vec<i64> {
    candidates
        .iter()
        .flat_map(|c| {
            [Some(c.author_id), c.retweeted_user_id, c.quoted_user_id]
                .into_iter()
                .flatten()
                .filter(|&id| id != 0)
                .map(|id| id as i64)
        })
        .collect::<HashSet<i64>>()
        .into_iter()
        .collect()
}

#[async_trait]
impl CachedHydrator<ScoredPostsQuery, PostCandidate> for GizmoduckCandidateHydrator {
    type CacheKey = GizmoduckCacheKey;
    type CacheValue = GizmoduckCacheValue;

    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        !query.has_cached_posts
    }

    fn cache_store(&self) -> &dyn CacheStore<Self::CacheKey, Self::CacheValue> {
        &self.cache
    }
    fn cache_key(&self, candidate: &PostCandidate) -> Self::CacheKey {
        GizmoduckCacheKey {
            author_id: candidate.author_id,
            retweeted_user_id: candidate.retweeted_user_id,
            quoted_user_id: candidate.quoted_user_id,
        }
    }

    fn cache_value(&self, hydrated: &PostCandidate) -> Self::CacheValue {
        GizmoduckCacheValue {
            author_followers_count: hydrated.author_followers_count,
            author_screen_name: hydrated.author_screen_name.clone(),
            retweeted_screen_name: hydrated.retweeted_screen_name.clone(),
            nsfw_author: hydrated.nsfw_author,
            nsfw_author_ads: hydrated.nsfw_author_ads,
            nsfw_author_phoenix: hydrated.nsfw_author_phoenix,
        }
    }

    fn hydrate_from_cache(&self, value: Self::CacheValue) -> PostCandidate {
        PostCandidate {
            author_followers_count: value.author_followers_count,
            author_screen_name: value.author_screen_name,
            retweeted_screen_name: value.retweeted_screen_name,
            nsfw_author: value.nsfw_author,
            nsfw_author_ads: value.nsfw_author_ads,
            nsfw_author_phoenix: value.nsfw_author_phoenix,
            ..Default::default()
        }
    }

    async fn hydrate_from_client(
        &self,
        _query: &ScoredPostsQuery,
        candidates: &[PostCandidate],
    ) -> Vec<Result<PostCandidate, String>> {
        let client = &self.gizmoduck_client;
        let user_ids_to_fetch = user_ids_to_fetch(candidates);
        let users = client.get_users(user_ids_to_fetch).await;

        candidates
            .iter()
            .map(|candidate| {
                let poster = lookup_user(&users, candidate.author_id);
                let original = candidate
                    .retweeted_user_id
                    .filter(|&id| id != 0)
                    .map(|id| lookup_user(&users, id));
                let quoted = candidate
                    .quoted_user_id
                    .filter(|&id| id != 0)
                    .map(|id| lookup_user(&users, id));
                let hydrated = hydrate_author_features(poster, original, quoted);
                Ok(PostCandidate {
                    author_followers_count: hydrated.author_followers_count,
                    author_screen_name: hydrated.author_screen_name,
                    retweeted_screen_name: hydrated.retweeted_screen_name,
                    nsfw_author: hydrated.nsfw_author,
                    nsfw_author_ads: hydrated.nsfw_author_ads,
                    nsfw_author_phoenix: hydrated.nsfw_author_phoenix,
                    ..Default::default()
                })
            })
            .collect()
    }

    fn update(&self, candidate: &mut PostCandidate, hydrated: PostCandidate) {
        candidate.author_followers_count = hydrated.author_followers_count;
        candidate.author_screen_name = hydrated.author_screen_name;
        candidate.retweeted_screen_name = hydrated.retweeted_screen_name;
        candidate.nsfw_author = hydrated.nsfw_author;
        candidate.nsfw_author_ads = hydrated.nsfw_author_ads;
        candidate.nsfw_author_phoenix = hydrated.nsfw_author_phoenix;
    }
}

fn lookup_user<E>(
    users: &HashMap<i64, Result<Option<GizmoduckUserResult>, E>>,
    user_id: u64,
) -> AuthorLookup {
    match users.get(&(user_id as i64)) {
        Some(Ok(Some(result))) => match result.user.as_ref() {
            Some(user) => {
                let nsfw = user.safety.nsfw_admin
                    || user.safety.nsfw_user
                    || user
                        .labels
                        .labels
                        .iter()
                        .any(|label| label.label_value == LabelValue::NSFW_HIGH_PRECISION.0);
                let nsfw_ads = nsfw
                    || user
                        .labels
                        .labels
                        .iter()
                        .any(|label| label.label_value == LabelValue::POSSIBLY_NSFW_ACCOUNT.0);
                AuthorLookup::Found {
                    followers_count: Some(user.counts.followers_count as i32),
                    screen_name: Some(user.profile.screen_name.clone()),
                    nsfw,
                    nsfw_ads,
                }
            }
            None => AuthorLookup::ConfirmedMissing,
        },
        Some(Ok(None)) => AuthorLookup::ConfirmedMissing,
        Some(Err(_)) | None => AuthorLookup::Unknown,
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GizmoduckCacheKey {
    pub author_id: u64,
    pub retweeted_user_id: Option<u64>,
    pub quoted_user_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct GizmoduckCacheValue {
    pub author_followers_count: Option<i32>,
    pub author_screen_name: Option<String>,
    pub retweeted_screen_name: Option<String>,
    pub nsfw_author: Option<bool>,
    pub nsfw_author_ads: Option<bool>,
    pub nsfw_author_phoenix: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(followers: i32, nsfw: bool, nsfw_ads: bool) -> AuthorLookup {
        AuthorLookup::Found {
            followers_count: Some(followers),
            screen_name: Some(format!("u{followers}")),
            nsfw,
            nsfw_ads,
        }
    }

    #[test]
    fn retweet_uses_original_author_follower_count() {
        let out = hydrate_author_features(
            found(50, false, false),
            Some(found(1_000_000, false, false)),
            None,
        );
        assert_eq!(out.author_followers_count, Some(1_000_000));
        assert_eq!(out.author_screen_name.as_deref(), Some("u50"));
        assert_eq!(out.retweeted_screen_name.as_deref(), Some("u1000000"));
        assert_eq!(out.nsfw_author, Some(false));
        assert_eq!(out.nsfw_author_phoenix, Some(false));
    }

    #[test]
    fn original_post_uses_poster_follower_count() {
        let out = hydrate_author_features(found(12_000, false, false), None, None);
        assert_eq!(out.author_followers_count, Some(12_000));
        assert_eq!(out.nsfw_author, Some(false));
    }

    #[test]
    fn quote_keeps_quoter_follower_count() {
        let out = hydrate_author_features(
            found(80, false, false),
            None,
            Some(found(2_000_000, false, false)),
        );
        assert_eq!(out.author_followers_count, Some(80));
        assert_eq!(out.nsfw_author, Some(false));
    }

    #[test]
    fn nsfw_original_author_labels_retweet() {
        let out = hydrate_author_features(
            found(50, false, false),
            Some(found(1_000_000, true, true)),
            None,
        );
        assert_eq!(out.nsfw_author, Some(true));
        assert_eq!(out.nsfw_author_ads, Some(true));
        assert_eq!(out.nsfw_author_phoenix, Some(true));
        assert_eq!(out.author_followers_count, Some(1_000_000));
    }

    #[test]
    fn nsfw_quoted_author_labels_quote() {
        let out =
            hydrate_author_features(found(80, false, false), None, Some(found(9, true, true)));
        assert_eq!(out.author_followers_count, Some(80));
        assert_eq!(out.nsfw_author, Some(true));
        assert_eq!(out.nsfw_author_phoenix, Some(true));
    }

    #[test]
    fn store_miss_on_original_author_fails_closed() {
        let out =
            hydrate_author_features(found(50, false, false), Some(AuthorLookup::Unknown), None);
        assert_eq!(out.nsfw_author, Some(true));
        assert_eq!(out.nsfw_author_ads, Some(true));
        assert_eq!(out.nsfw_author_phoenix, Some(true));
        assert_eq!(out.author_followers_count, None);
    }

    #[test]
    fn store_miss_on_quoted_author_fails_closed() {
        let out =
            hydrate_author_features(found(80, false, false), None, Some(AuthorLookup::Unknown));
        assert_eq!(out.nsfw_author, Some(true));
        assert_eq!(out.author_followers_count, Some(80));
    }

    #[test]
    fn store_miss_on_poster_fails_closed() {
        let out = hydrate_author_features(AuthorLookup::Unknown, None, None);
        assert_eq!(out.nsfw_author, Some(true));
        assert_eq!(out.nsfw_author_ads, Some(true));
        assert_eq!(out.nsfw_author_phoenix, Some(true));
        assert_eq!(out.author_followers_count, None);
    }

    #[test]
    fn confirmed_missing_original_is_not_a_store_miss() {
        let out = hydrate_author_features(
            found(50, false, false),
            Some(AuthorLookup::ConfirmedMissing),
            None,
        );
        assert_eq!(out.nsfw_author, Some(false));
        assert_eq!(out.author_followers_count, None);
    }

    #[test]
    fn fetch_ids_include_retweeted_and_quoted_authors() {
        let ids = user_ids_to_fetch(&[PostCandidate {
            author_id: 1,
            retweeted_user_id: Some(2),
            quoted_user_id: Some(3),
            ..Default::default()
        }]);
        let set: HashSet<i64> = ids.into_iter().collect();
        assert_eq!(set, HashSet::from([1, 2, 3]));
    }

    #[test]
    fn cache_key_includes_quoted_and_retweeted_ids() {
        let key = GizmoduckCacheKey {
            author_id: 1,
            retweeted_user_id: Some(2),
            quoted_user_id: Some(3),
        };
        assert_ne!(
            key,
            GizmoduckCacheKey {
                author_id: 1,
                retweeted_user_id: Some(2),
                quoted_user_id: None,
            }
        );
    }
}
