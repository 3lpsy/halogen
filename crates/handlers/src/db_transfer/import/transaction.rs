use halogen_orm::{
    episode, episode_chapter, episode_playlist, playback, playlist, podcast, podcast_auto_playlist,
    podcast_config, user, user_episode_status, user_podcast,
};
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::DatabaseTransaction;
use std::collections::HashMap;
#[allow(clippy::too_many_arguments)]
pub(super) async fn merge_in_txn(
    txn: &DatabaseTransaction,
    src_users: Vec<user::Model>,
    src_podcasts: Vec<podcast::Model>,
    src_configs: HashMap<i32, podcast_config::Model>,
    src_episodes: Vec<episode::Model>,
    src_chapters: Vec<episode_chapter::Model>,
    src_subscriptions: Vec<user_podcast::Model>,
    src_playbacks: Vec<playback::Model>,
    src_statuses: Vec<user_episode_status::Model>,
    src_playlists: Vec<playlist::Model>,
    src_links: Vec<episode_playlist::Model>,
    src_auto: Vec<podcast_auto_playlist::Model>,
) -> Result<DbImportSummaryData, ValidationErrors> {
    let mut summary = DbImportSummaryData::default();

    let user_map = super::users::merge(txn, &src_users, &mut summary).await?;
    let (podcast_map, merged_podcasts) =
        super::podcasts::merge(txn, &src_podcasts, &src_configs, &user_map, &mut summary).await?;
    super::subscriptions::merge(
        txn,
        &src_subscriptions,
        &user_map,
        &podcast_map,
        &mut summary,
    )
    .await?;
    let episode_map = super::episodes::merge(
        txn,
        &src_episodes,
        &src_chapters,
        &podcast_map,
        &merged_podcasts,
        &mut summary,
    )
    .await?;
    super::playbacks::merge(txn, &src_playbacks, &user_map, &episode_map, &mut summary).await?;
    super::statuses::merge(txn, &src_statuses, &user_map, &episode_map, &mut summary).await?;
    let playlist_map =
        super::playlists::merge(txn, &src_playlists, &user_map, &mut summary).await?;
    super::memberships::merge(txn, &src_links, &playlist_map, &episode_map, &mut summary).await?;
    super::auto_playlists::merge(txn, &src_auto, &podcast_map, &playlist_map, &mut summary).await?;
    Ok(summary)
}
