use crate::clients::who_to_follow_client::WhoToFollowClient;
use crate::models::query::{RequestType, ScoredPostsQuery};
use crate::params::EnableWhoToFollowModule;
use std::collections::HashSet;
use std::sync::Arc;
use tonic::async_trait;
use xai_account_recommendations_mixer_proto::{
    product_context, AccountRecommendationsMixerRequest, ClientContext,
    HomeReverseChronWhoToFollowProductContext, HomeWhoToFollowProductContext, Product,
    ProductContext, WhoToFollowReactiveContext,
};
use xai_candidate_pipeline::source::Source;
use xai_home_mixer_proto::{feed_item, FeedItem, WhoToFollowModule};
use xai_x_thrift::served_history::EntityIdType;

const EXCLUDED_USER_IDS_LIMIT: usize = 200;
const MAX_WHO_TO_FOLLOW_USERS: usize = 3;

pub struct WhoToFollowSource {
    pub who_to_follow_client: Arc<dyn WhoToFollowClient + Send + Sync>,
}

#[async_trait]
impl Source<ScoredPostsQuery, FeedItem> for WhoToFollowSource {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        query.params.get(EnableWhoToFollowModule)
            && query.who_to_follow_eligible
            && query.mute_block_lists_ready()
    }

    async fn source(&self, query: &ScoredPostsQuery) -> Result<Vec<FeedItem>, String> {
        let request = build_wtf_request(query);

        let mut response = self
            .who_to_follow_client
            .get_wtf_recommendations(request)
            .await
            .map_err(|e| format!("WhoToFollowSource: {e}"))?;

        if response.user_recommendations.is_empty() {
            return Ok(vec![]);
        }

        response
            .user_recommendations
            .truncate(MAX_WHO_TO_FOLLOW_USERS);

        let module = WhoToFollowModule {
            who_to_follow_response: Some(response),
        };

        Ok(vec![FeedItem {
            position: 0,
            item: Some(feed_item::Item::WhoToFollow(module)),
        }])
    }
}

fn build_wtf_request(query: &ScoredPostsQuery) -> AccountRecommendationsMixerRequest {
    let excluded_user_ids = get_excluded_user_ids(query);
    let reactive = WhoToFollowReactiveContext {
        excluded_user_ids,
        followed_user_id: None,
        dismissed_user_id: None,
    };

    let (product, product_context) = match query.request_type {
        RequestType::Following => (
            Product::HomeReverseChronWhoToFollow,
            ProductContext {
                context: Some(
                    product_context::Context::HomeReverseChronWhoToFollowProductContext(
                        HomeReverseChronWhoToFollowProductContext {
                            wtf_reactive_context: Some(reactive),
                        },
                    ),
                ),
            },
        ),
        _ => (
            Product::HomeWhoToFollow,
            ProductContext {
                context: Some(product_context::Context::HomeWhoToFollowProductContext(
                    HomeWhoToFollowProductContext {
                        wtf_reactive_context: Some(reactive),
                    },
                )),
            },
        ),
    };

    AccountRecommendationsMixerRequest {
        client_context: Some(ClientContext {
            user_id: Some(query.user_id as i64),
            app_id: Some(query.client_app_id as i64),
            country_code: Some(query.country_code.clone()),
            language_code: Some(query.language_code.clone()),
            ip_address: Some(query.ip_address.clone()),
            user_agent: Some(query.user_agent.clone()),
            ..Default::default()
        }),
        product: product as i32,
        debug_params: None,
        cursor: None,
        product_context: Some(product_context),
    }
}

fn get_excluded_user_ids(query: &ScoredPostsQuery) -> Vec<i64> {
    let mut ids = Vec::with_capacity(EXCLUDED_USER_IDS_LIMIT);
    let mut seen = HashSet::new();

    // Mute/block first so fatigue IDs cannot crowd a blocked account out of the cap.
    for id in query
        .user_features
        .blocked_user_ids
        .iter()
        .chain(query.user_features.muted_user_ids.iter())
        .copied()
    {
        if seen.insert(id) {
            ids.push(id);
            if ids.len() >= EXCLUDED_USER_IDS_LIMIT {
                return ids;
            }
        }
    }

    let fatigue = query
        .served_history
        .iter()
        .flat_map(|sh| &sh.entries)
        .filter(|entry| entry.entity_type == EntityIdType::WHO_TO_FOLLOW)
        .flat_map(|entry| {
            entry
                .item_ids
                .iter()
                .flatten()
                .filter_map(|item| item.user_id)
        });

    for id in fatigue {
        if seen.insert(id) {
            ids.push(id);
            if ids.len() >= EXCLUDED_USER_IDS_LIMIT {
                return ids;
            }
        }
    }

    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::who_to_follow_client::MockWhoToFollowClient;
    use crate::models::user_features::UserFeatures;
    use xai_x_thrift::served_history::{
        EntityIdType, EntryWithItemIds, ItemIds, ServedHistory, ServedRequestType,
    };

    fn source() -> WhoToFollowSource {
        WhoToFollowSource {
            who_to_follow_client: Arc::new(MockWhoToFollowClient),
        }
    }

    fn fatigue_history(user_id: i64) -> Vec<ServedHistory> {
        vec![ServedHistory {
            request_type: ServedRequestType::OLDER,
            served_id: None,
            served_time_ms: Some(1),
            entries: vec![EntryWithItemIds {
                entity_type: EntityIdType::WHO_TO_FOLLOW,
                sort_index: None,
                size: None,
                item_ids: Some(vec![ItemIds {
                    user_id: Some(user_id),
                    tweet_id: None,
                    source_tweet_id: None,
                    quote_tweet_id: None,
                    source_author_id: None,
                    quote_author_id: None,
                    in_reply_to_tweet_id: None,
                    in_reply_to_author_id: None,
                    article_id: None,
                    tweet_score: None,
                    entry_id_to_replace: None,
                    impression_id: None,
                }]),
            }],
        }]
    }

    #[test]
    fn enable_requires_mute_and_block_lists() {
        let src = source();
        let mut query = ScoredPostsQuery {
            who_to_follow_eligible: true,
            ..Default::default()
        };
        assert!(!src.enable(&query));

        query.muted_user_ids_hydrated = true;
        assert!(!src.enable(&query));

        query.blocked_user_ids_hydrated = true;
        assert!(src.enable(&query));
    }

    #[test]
    fn excludes_muted_and_blocked_before_fatigue() {
        let query = ScoredPostsQuery {
            user_features: UserFeatures {
                blocked_user_ids: vec![10],
                muted_user_ids: vec![20],
                ..Default::default()
            },
            served_history: fatigue_history(30),
            ..Default::default()
        };
        let ids = get_excluded_user_ids(&query);
        assert_eq!(ids, vec![10, 20, 30]);
    }

    #[test]
    fn mute_block_ids_win_the_cap_over_fatigue() {
        let query = ScoredPostsQuery {
            user_features: UserFeatures {
                blocked_user_ids: (1..=EXCLUDED_USER_IDS_LIMIT as i64).collect(),
                muted_user_ids: vec![9_999],
                ..Default::default()
            },
            served_history: fatigue_history(8_888),
            ..Default::default()
        };
        let ids = get_excluded_user_ids(&query);
        assert_eq!(ids.len(), EXCLUDED_USER_IDS_LIMIT);
        assert!(ids.contains(&1));
        assert!(ids.contains(&(EXCLUDED_USER_IDS_LIMIT as i64)));
        assert!(!ids.contains(&8_888));
        assert!(!ids.contains(&9_999));
    }
}
