use std::collections::HashMap;

use halogen_wire::{DefaultListParams, Paginator, PodcastData, PodcastInclude, ValidationErrors};
use sea_orm::sea_query::{Expr, Func, SimpleExpr};
use sea_orm::{ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect};
use tracing::info;

use crate::handlers::{db_error, wants};
use halogen_orm::episode as episode_entity;
use halogen_orm::podcast::{Column, Entity as PodcastEntity};
use halogen_orm::podcast_config as podcast_config_entity;
use halogen_orm::user_podcast;

/// One `(podcast_id, count)` row from the grouped episode count query.
#[derive(FromQueryResult)]
struct PodcastEpisodeCount {
    podcast_id: i32,
    count: i64,
}

/// Episodes-per-podcast for the given podcast ids, in one grouped query.
pub(crate) async fn episode_counts(
    dbc: &sea_orm::DatabaseConnection,
    podcast_ids: &[i32],
) -> Result<HashMap<i32, u64>, ValidationErrors> {
    if podcast_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = episode_entity::Entity::find()
        .select_only()
        .column(episode_entity::Column::PodcastId)
        .column_as(
            SimpleExpr::from(Func::count(Expr::col(episode_entity::Column::Id))),
            "count",
        )
        .filter(episode_entity::Column::PodcastId.is_in(podcast_ids.to_vec()))
        .group_by(episode_entity::Column::PodcastId)
        .into_model::<PodcastEpisodeCount>()
        .all(dbc)
        .await
        .map_err(db_error("counting podcast episodes"))?;
    // COUNT() comes back signed from SQL; it can't actually be negative.
    Ok(rows
        .into_iter()
        .map(|r| (r.podcast_id, u64::try_from(r.count).unwrap_or(0)))
        .collect())
}

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    params: &DefaultListParams<PodcastInclude>,
) -> Result<(Vec<PodcastData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    let order = params.order.clone().unwrap_or_default();
    let load_podcast_config = wants(params.includes.as_ref(), PodcastInclude::PodcastConfig);

    // Scope to the caller's subscriptions: a user only sees podcasts they are
    // subscribed to (via `user_podcast`).
    let subscribed: Vec<i32> = user_podcast::Entity::find()
        .filter(user_podcast::Column::UserId.eq(user_id))
        .select_only()
        .column(user_podcast::Column::PodcastId)
        .into_tuple::<i32>()
        .all(dbc)
        .await
        .map_err(db_error("loading subscriptions"))?;

    let mut query = PodcastEntity::find().filter(Column::Id.is_in(subscribed));
    // Optional id narrowing (e.g. the client batch-priming the pool for the
    // podcasts an episode page references — one request instead of N). ANDed onto
    // the subscription scope above, so it can only ever narrow, never widen, what
    // the caller may see.
    if let Some(ids) = params.filter.as_ref().and_then(|f| f.ids.as_ref())
        && !ids.is_empty()
    {
        query = query.filter(Column::Id.is_in(ids.clone()));
    }
    let (podcasts, paginator) =
        halogen_orm::common::paginate(dbc, query, &pagination, &order).await?;

    // Episodes-per-podcast in one grouped query (so the UI never holds the whole
    // episode table just to show counts).
    let podcast_ids: Vec<i32> = podcasts.iter().map(|p| p.id).collect();
    let counts = episode_counts(dbc, &podcast_ids).await?;

    let mut responses: Vec<PodcastData> = if load_podcast_config && !podcasts.is_empty() {
        let config_ids: Vec<_> = podcasts
            .iter()
            .filter_map(|p| p.podcast_config_id)
            .collect();

        let configs: HashMap<_, _> = if !config_ids.is_empty() {
            podcast_config_entity::Entity::find()
                .filter(podcast_config_entity::Column::Id.is_in(config_ids))
                .all(dbc)
                .await
                .map_err(db_error("loading podcast configs"))?
                .into_iter()
                .map(|c| (c.id, c))
                .collect()
        } else {
            HashMap::new()
        };

        podcasts
            .into_iter()
            .map(|podcast| {
                let podcast_config_id = podcast.podcast_config_id;
                let mut data: PodcastData = podcast.into();
                data.podcast_config = podcast_config_id
                    .and_then(|id| configs.get(&id).cloned())
                    .map(|c| c.into());
                data
            })
            .collect()
    } else {
        podcasts.into_iter().map(PodcastData::from).collect()
    };

    for r in responses.iter_mut() {
        r.episode_count = counts.get(&r.id).copied();
    }

    info!(
        "Fetched {} podcasts (page {}, size {})",
        responses.len(),
        pagination.page,
        pagination.size
    );
    Ok((responses, paginator))
}
