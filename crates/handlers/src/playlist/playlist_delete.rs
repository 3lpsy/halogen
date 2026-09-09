use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter, TransactionTrait};
use tracing::info;

use crate::{db_error, not_found};
use halogen_orm::episode_playlist::{Column as EpisodeColumn, Entity as EpisodePlaylistEntity};
use halogen_orm::playlist::{Column as PlaylistColumn, Entity as PlaylistEntity};
use halogen_orm::podcast_auto_playlist::{
    Column as AutoPlaylistColumn, Entity as PodcastAutoPlaylistEntity,
};
use halogen_wire::ValidationErrors;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: Option<i32>,
    playlist_id: i32,
) -> Result<(), ValidationErrors> {
    // Defense-in-depth: the router runs the ownership guard, but scope the lookup
    // by `user_id` too so a non-owner can never delete (mirrors playback delete).
    // `None` = admin (guard already granted the bypass), so no owner filter.
    let mut find = PlaylistEntity::find_by_id(playlist_id);
    if let Some(uid) = user_id {
        find = find.filter(PlaylistColumn::UserId.eq(uid));
    }
    let playlist_model = find
        .one(dbc)
        .await
        .map_err(db_error("fetching playlist"))?
        .ok_or_else(|| not_found("Playlist not found"))?;

    // All three deletes in one transaction: a mid-way failure must not leave a
    // playlist whose memberships were already removed (partial corruption).
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    EpisodePlaylistEntity::delete_many()
        .filter(EpisodeColumn::PlaylistId.eq(playlist_model.id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting playlist associations"))?;

    // Podcast auto-add links pointing at this playlist. FKs are ON, so deleting the
    // playlist would cascade these — we remove them explicitly for deterministic
    // cleanup rather than relying on the cascade.
    PodcastAutoPlaylistEntity::delete_many()
        .filter(AutoPlaylistColumn::PlaylistId.eq(playlist_model.id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting podcast auto-playlist links"))?;

    playlist_model
        .delete(&txn)
        .await
        .map_err(db_error("deleting playlist"))?;

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;

    info!("Deleted playlist");
    Ok(())
}
