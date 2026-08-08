use halogen_wire::{EpisodeData, EpisodeInclude, ValidationErrors};
use sea_orm::DatabaseConnection;
use tracing::info;

use crate::handlers::wants;
use halogen_orm::common::EntityHelpers;
use halogen_orm::episode::Entity as EpisodeEntity;

use super::user_playback::user_playback_for;
use super::user_status::user_status_for;

pub async fn handle(
    dbc: &DatabaseConnection,
    user_id: i32,
    episode_id: i32,
    includes: Option<&Vec<EpisodeInclude>>,
) -> Result<EpisodeData, ValidationErrors> {
    let episode = EpisodeEntity::by_id_or_err(dbc, episode_id).await?;

    let load_podcast = wants(includes, EpisodeInclude::Podcast);
    let load_playback = wants(includes, EpisodeInclude::Playback);
    let load_chapters = wants(includes, EpisodeInclude::Chapters);

    let mut data = EpisodeData::from(episode);

    // Per-user listen state (the `From<Model>` defaulted this to Unplayed).
    data.playback_status = user_status_for(dbc, user_id, episode_id).await;

    // Embed the caller's resume cursor when requested (scoped to this user).
    if load_playback {
        data.playback = user_playback_for(dbc, user_id, episode_id).await;
    }

    // Embed the episode's chapter markers when requested (read-only, shared).
    if load_chapters {
        data.chapters = Some(super::chapters::chapters_for(dbc, episode_id).await);
    }

    super::attach_podcasts(dbc, std::slice::from_mut(&mut data), load_podcast).await?;

    info!("Fetched episode '{}'", data.title);
    Ok(data)
}
