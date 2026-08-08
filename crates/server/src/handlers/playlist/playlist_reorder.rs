use std::cmp::Ordering;
use std::collections::HashMap;

use halogen_orm::episode::{Column as EpisodeColumn, Entity as EpisodeEntity};
use halogen_orm::episode_playlist::{Column, Entity as EpisodePlaylistEntity, Model};
use halogen_wire::{OrderDirection, PlaylistReorderField, ValidationErrors, cmp_opt};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::info;

use crate::handlers::db_error;
use crate::handlers::playlist::episode_playlist::rewrite_positions;

/// Smart-reorder a playlist: rewrite `episode_playlist.position` so the members
/// are ordered by `field` in `direction`. Episode-intrinsic fields
/// (`Published`/`Title`/`Duration`) sort by the episode row; `Added` sorts by the
/// pivot's `created_at` (when the episode joined THIS playlist). Absent values
/// (nulls) sort last regardless of direction, and ties break by episode id, so the
/// result is stable and reproducible. A playlist with <2 members is a no-op.
pub async fn handle_reorder(
    dbc: &sea_orm::DatabaseConnection,
    playlist_id: i32,
    field: PlaylistReorderField,
    direction: OrderDirection,
) -> Result<(), ValidationErrors> {
    let now = chrono::Utc::now();

    let rows = EpisodePlaylistEntity::find()
        .filter(Column::PlaylistId.eq(playlist_id))
        .all(dbc)
        .await
        .map_err(db_error("fetching playlist membership for reorder"))?;
    if rows.len() < 2 {
        return Ok(());
    }

    let ordered = sorted_episode_ids(dbc, &rows, field, &direction).await?;
    rewrite_positions(dbc, playlist_id, &ordered, now).await?;

    info!(playlist_id, ?field, ?direction, "Smart-reordered playlist");
    Ok(())
}

/// The membership episode ids sorted by `field`/`direction`. `Added` reads the
/// pivot rows; everything else loads the episode rows for the sortable columns.
async fn sorted_episode_ids(
    dbc: &sea_orm::DatabaseConnection,
    rows: &[Model],
    field: PlaylistReorderField,
    direction: &OrderDirection,
) -> Result<Vec<i32>, ValidationErrors> {
    let mut ids: Vec<i32> = rows.iter().map(|r| r.episode_id).collect();

    if field == PlaylistReorderField::Added {
        let added: HashMap<i32, chrono::DateTime<chrono::Utc>> =
            rows.iter().map(|r| (r.episode_id, r.created_at)).collect();
        ids.sort_by(|a, b| {
            cmp_opt(added.get(a).copied(), added.get(b).copied(), direction).then(a.cmp(b))
        });
        return Ok(ids);
    }

    let episodes = EpisodeEntity::find()
        .filter(EpisodeColumn::Id.is_in(ids.clone()))
        .all(dbc)
        .await
        .map_err(db_error("loading episodes for reorder"))?;
    let by_id: HashMap<i32, halogen_orm::episode::Model> =
        episodes.into_iter().map(|e| (e.id, e)).collect();

    ids.sort_by(|a, b| {
        let ea = by_id.get(a);
        let eb = by_id.get(b);
        let ord = match field {
            PlaylistReorderField::Published => cmp_opt(
                ea.and_then(|e| e.published_at),
                eb.and_then(|e| e.published_at),
                direction,
            ),
            PlaylistReorderField::Duration => cmp_opt(
                ea.and_then(|e| e.duration_secs),
                eb.and_then(|e| e.duration_secs),
                direction,
            ),
            PlaylistReorderField::Title => cmp_opt(
                ea.map(|e| e.title.to_lowercase()),
                eb.map(|e| e.title.to_lowercase()),
                direction,
            ),
            // Handled by the early return above.
            PlaylistReorderField::Added => Ordering::Equal,
        };
        ord.then(a.cmp(b))
    });
    Ok(ids)
}
