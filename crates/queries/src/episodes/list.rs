use halogen_orm::{
    episode::{Column, Entity as EpisodeEntity},
    user_episode_status as ues_entity, user_podcast,
};
use halogen_wire::{DefaultListParams, EpisodeInclude, PlaybackStatus, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use validator::Validate;

/// Build an episode query scoped to the actor's subscriptions and listen state.
pub async fn scoped_query(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    params: &DefaultListParams<EpisodeInclude>,
) -> Result<sea_orm::Select<EpisodeEntity>, ValidationErrors> {
    if user_id < 1 {
        return Err(halogen_utils::verrors(
            "user_id",
            "range",
            "Invalid actor".into(),
        ));
    }
    params.validate()?;
    // Scope to the caller's subscriptions: a user only sees episodes from podcasts they're subscribed to (via
    // `user_podcast`). Mirrors `podcast_list`, which scopes the podcast list the same way. Without this base
    // filter, `GET /episodes` (and `GET /podcasts/{id}/episodes`, which delegates here) would leak every
    // owner's episodes across the whole server.
    let subscribed: Vec<i32> = user_podcast::Entity::find()
        .filter(user_podcast::Column::UserId.eq(user_id))
        .select_only()
        .column(user_podcast::Column::PodcastId)
        .into_tuple::<i32>()
        .all(dbc)
        .await
        .map_err(halogen_wire::DbValidationErrors::from)?;

    let mut query = EpisodeEntity::find().filter(Column::PodcastId.is_in(subscribed));

    // Filters, including multi-column search and its relevance ranking (which
    // orders *before* the requested sort applied just below).
    if let Some(ref filter) = params.filter {
        query = super::filters::apply_episode_filters(query, filter);

        // Per-user playback-status filter. The status moved off `episode` into the
        // per-user `user_episode_status` table, so it can't be a plain column
        // predicate; we resolve it against the caller's rows and constrain the
        // episode id set accordingly. Episodes with no row count as UNPLAYED.
        if let Some(ref status) = filter.playback_status {
            // Map the raw filter string to a known status. An unknown/garbage
            // value matches nothing — preserving the pre-refactor behaviour where
            // a column-equality on a bad value simply never matched.
            let wanted = match status.as_str() {
                "UNPLAYED" => Some(PlaybackStatus::Unplayed),
                "PLAYED" => Some(PlaybackStatus::Played),
                "FINISHED" => Some(PlaybackStatus::Finished),
                _ => None,
            };
            match wanted {
                None => {
                    query = query.filter(Column::Id.is_in(Vec::<i32>::new()));
                }
                Some(PlaybackStatus::Unplayed) => {
                    // Unplayed = no row OR a row still marked Unplayed → exclude
                    // every episode the caller has marked PLAYED/FINISHED.
                    let non_unplayed: Vec<i32> = ues_entity::Entity::find()
                        .filter(ues_entity::Column::UserId.eq(user_id))
                        .filter(ues_entity::Column::PlaybackStatus.ne(PlaybackStatus::Unplayed))
                        .select_only()
                        .column(ues_entity::Column::EpisodeId)
                        .into_tuple::<i32>()
                        .all(dbc)
                        .await
                        .map_err(halogen_wire::DbValidationErrors::from)?;
                    if !non_unplayed.is_empty() {
                        query = query.filter(Column::Id.is_not_in(non_unplayed));
                    }
                }
                Some(wanted) => {
                    // PLAYED / FINISHED: restrict to episodes whose per-user row
                    // has that exact status. No matches → an empty id set.
                    let matching: Vec<i32> = ues_entity::Entity::find()
                        .filter(ues_entity::Column::UserId.eq(user_id))
                        .filter(ues_entity::Column::PlaybackStatus.eq(wanted))
                        .select_only()
                        .column(ues_entity::Column::EpisodeId)
                        .into_tuple::<i32>()
                        .all(dbc)
                        .await
                        .map_err(halogen_wire::DbValidationErrors::from)?;
                    query = query.filter(Column::Id.is_in(matching));
                }
            }
        }
    }

    Ok(query)
}
