use crate::clients::served_history_client::{ServedHistoryClient, TimelineType};
use crate::models::query::ScoredPostsQuery;
use crate::params::{
    EnableUrtMigrationComponents, ExcludeServedTweetIdsDuration, ExcludeServedTweetIdsNumber,
    FeedSurveyFatigueHours, WhoToFollowFatigueHours,
};
use std::sync::Arc;
use tonic::async_trait;
use xai_candidate_pipeline::component_library::utils::client_utils::app_id_to_served_history_id;
use xai_candidate_pipeline::query_hydrator::QueryHydrator;
use xai_x_thrift::served_history::{EntityIdType, ServedHistory};

pub struct ServedHistoryQueryHydrator {
    client: Arc<dyn ServedHistoryClient>,
}

impl ServedHistoryQueryHydrator {
    pub fn from_client(client: Arc<dyn ServedHistoryClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl QueryHydrator<ScoredPostsQuery> for ServedHistoryQueryHydrator {
    fn enable(&self, query: &ScoredPostsQuery) -> bool {
        query.params.get(EnableUrtMigrationComponents)
    }

    async fn hydrate(&self, query: &ScoredPostsQuery) -> Result<ScoredPostsQuery, String> {
        let platform = app_id_to_served_history_id(query.client_app_id);
        let timeline_type = TimelineType::for_request(query.request_type);
        let entries = self
            .client
            .get_recent(query.user_id, timeline_type, platform)
            .await
            .map_err(|e| e.to_string())?;

        let who_to_follow_eligible = is_module_eligible(
            &entries,
            EntityIdType::WHO_TO_FOLLOW,
            query.params.get(WhoToFollowFatigueHours),
            query.request_time_ms,
        );

        let feed_survey_eligible = is_module_eligible(
            &entries,
            EntityIdType::ANNOTATION,
            query.params.get(FeedSurveyFatigueHours),
            query.request_time_ms,
        );

        let served_ids = recently_served_ids(
            &entries,
            query.request_time_ms,
            query.params.get(ExcludeServedTweetIdsDuration),
            query.params.get(ExcludeServedTweetIdsNumber),
        );

        Ok(ScoredPostsQuery {
            served_history: entries,
            served_ids,
            who_to_follow_eligible,
            feed_survey_eligible,
            ..Default::default()
        })
    }

    fn update(&self, query: &mut ScoredPostsQuery, hydrated: ScoredPostsQuery) {
        query.served_history = hydrated.served_history;
        query.served_ids = hydrated.served_ids;
        query.who_to_follow_eligible = hydrated.who_to_follow_eligible;
        query.feed_survey_eligible = hydrated.feed_survey_eligible;
    }
}

fn recently_served_ids(
    history: &[ServedHistory],
    now_ms: i64,
    duration_minutes: u32,
    max_items: usize,
) -> Vec<u64> {
    let min_time_ms = now_ms - (duration_minutes as i64 * 60_000);

    // Organic exclude-already-served is TWEET-only. Ads are written as
    // PROMOTED_TWEET with tweet_id = ad.post_id; folding those ids into
    // served_ids lets one ad impression bury the organic post and also
    // consume the max_items budget so recent organic ids fall off.
    history
        .iter()
        .filter(|sh| sh.served_time_ms.is_some_and(|t| t >= min_time_ms))
        .flat_map(|sh| sh.entries.iter())
        .filter(|entry| entry.entity_type == EntityIdType::TWEET)
        .flat_map(|entry| {
            entry.item_ids.iter().flatten().flat_map(|ids| {
                [ids.tweet_id, ids.source_tweet_id]
                    .into_iter()
                    .flatten()
                    .map(|id| id as u64)
            })
        })
        .take(max_items)
        .collect()
}

fn is_module_eligible(
    history: &[ServedHistory],
    entity_type: EntityIdType,
    fatigue_hours: u32,
    now_ms: i64,
) -> bool {
    let min_interval_ms = fatigue_hours as i64 * 3_600_000;

    let last_served_ms = history
        .iter()
        .filter(|sh| sh.entries.iter().any(|e| e.entity_type == entity_type))
        .filter_map(|sh| sh.served_time_ms)
        .max();

    match last_served_ms {
        Some(ts) => (now_ms - ts) >= min_interval_ms,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::previously_served_posts_filter::PreviouslyServedPostsFilter;
    use crate::models::candidate::PostCandidate;
    use xai_candidate_pipeline::filter::Filter;
    use xai_x_thrift::served_history::{
        EntryWithItemIds, ItemIds, RequestType as ServedRequestType,
    };

    fn tweet_ids(tweet_id: Option<i64>, source_tweet_id: Option<i64>) -> Option<Vec<ItemIds>> {
        Some(vec![ItemIds {
            tweet_id,
            source_tweet_id,
            quote_tweet_id: None,
            source_author_id: None,
            quote_author_id: None,
            in_reply_to_tweet_id: None,
            in_reply_to_author_id: None,
            article_id: None,
            tweet_score: None,
            entry_id_to_replace: None,
            user_id: None,
            impression_id: None,
        }])
    }

    fn entry(entity_type: EntityIdType, tweet_id: i64) -> EntryWithItemIds {
        EntryWithItemIds {
            entity_type,
            sort_index: Some(0),
            size: None,
            item_ids: tweet_ids(Some(tweet_id), None),
        }
    }

    fn history(served_time_ms: i64, entries: Vec<EntryWithItemIds>) -> ServedHistory {
        ServedHistory {
            request_type: ServedRequestType::INITIAL,
            entries,
            served_id: Some(1),
            served_time_ms: Some(served_time_ms),
        }
    }

    fn organic(tweet_id: u64) -> PostCandidate {
        PostCandidate {
            tweet_id,
            ..Default::default()
        }
    }

    #[test]
    fn organic_tweet_ids_enter_served_ids() {
        let ids = recently_served_ids(
            &[history(9_000, vec![entry(EntityIdType::TWEET, 11)])],
            10_000,
            10,
            100,
        );
        assert_eq!(ids, vec![11]);
    }

    #[test]
    fn promoted_tweet_ids_do_not_enter_organic_served_ids() {
        let ids = recently_served_ids(
            &[history(
                9_000,
                vec![
                    entry(EntityIdType::TWEET, 11),
                    entry(EntityIdType::PROMOTED_TWEET, 22),
                ],
            )],
            10_000,
            10,
            100,
        );
        assert_eq!(ids, vec![11]);
        assert!(!ids.contains(&22));
    }

    #[test]
    fn ads_do_not_consume_the_organic_served_id_budget() {
        let ads: Vec<EntryWithItemIds> = (1..=5)
            .map(|i| entry(EntityIdType::PROMOTED_TWEET, 100 + i))
            .collect();
        let organics: Vec<EntryWithItemIds> = (1..=3)
            .map(|i| entry(EntityIdType::TWEET, i))
            .collect();
        let mut entries = ads;
        entries.extend(organics);

        let ids = recently_served_ids(&[history(9_000, entries)], 10_000, 10, 3);
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn retweet_source_id_on_organic_entries_is_kept() {
        let ids = recently_served_ids(
            &[history(
                9_000,
                vec![EntryWithItemIds {
                    entity_type: EntityIdType::TWEET,
                    sort_index: Some(0),
                    size: None,
                    item_ids: tweet_ids(Some(50), Some(7)),
                }],
            )],
            10_000,
            10,
            100,
        );
        assert_eq!(ids, vec![50, 7]);
    }

    #[test]
    fn expired_history_is_ignored() {
        let ids = recently_served_ids(
            &[history(0, vec![entry(EntityIdType::TWEET, 11)])],
            10 * 60_000 + 1,
            10,
            100,
        );
        assert!(ids.is_empty());
    }

    #[test]
    fn previously_served_filter_keeps_organic_post_after_ad_impression() {
        let served_ids = recently_served_ids(
            &[history(
                9_000,
                vec![entry(EntityIdType::PROMOTED_TWEET, 22)],
            )],
            10_000,
            10,
            100,
        );
        let query = ScoredPostsQuery {
            served_ids,
            ..Default::default()
        };
        let result = PreviouslyServedPostsFilter.filter(&query, vec![organic(22)]);
        assert_eq!(
            result.kept.iter().map(|c| c.tweet_id).collect::<Vec<_>>(),
            vec![22]
        );
        assert!(result.removed.is_empty());
    }

    #[test]
    fn previously_served_filter_still_drops_organic_after_organic_impression() {
        let served_ids = recently_served_ids(
            &[history(9_000, vec![entry(EntityIdType::TWEET, 22)])],
            10_000,
            10,
            100,
        );
        let query = ScoredPostsQuery {
            served_ids,
            ..Default::default()
        };
        let result = PreviouslyServedPostsFilter.filter(&query, vec![organic(22)]);
        assert!(result.kept.is_empty());
        assert_eq!(
            result.removed.iter().map(|c| c.tweet_id).collect::<Vec<_>>(),
            vec![22]
        );
    }
}
