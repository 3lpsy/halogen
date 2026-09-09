use crate::SyncService;
use halogen_webui_store::{OutboxOp, StoreChanges};
use halogen_wire::{EpisodeInclude, PlaylistInclude};

impl SyncService {
    /// Repair rejected optimistic values once queued work no longer needs its local overlay.
    pub(crate) async fn repair_rejections(&mut self) {
        let Ok(entries) = self.store.journal_entries().await else {
            return;
        };
        if entries
            .iter()
            .any(|(_, row)| row.rejection.is_none() && !row.delivered)
        {
            return;
        }
        let now = halogen_webui_platform::time::now_ms();
        if self
            .last_repair_at_ms
            .is_some_and(|last| now >= last && now - last < 30_000)
        {
            return;
        }
        if !entries
            .iter()
            .any(|(id, row)| row.rejection.is_some() && !self.repaired_rejections.contains(id))
        {
            return;
        }
        self.last_repair_at_ms = Some(now);
        for (id, entry) in entries {
            if entry.rejection.is_none() || self.repaired_rejections.contains(&id) {
                continue;
            }
            match self.repair_operation(&entry.operation).await {
                Ok(()) => {
                    self.repaired_rejections.insert(id);
                }
                Err(error) => {
                    halogen_webui_logging::warn!(%error, "Rejected change cache repair will retry");
                }
            }
        }
    }

    async fn repair_operation(&mut self, operation: &OutboxOp) -> anyhow::Result<()> {
        let Some(api) = self.api_client.as_ref().map(|api| api.clone_handle()) else {
            anyhow::bail!("no authenticated client");
        };
        let mut episodes = Vec::new();
        let mut playlists = Vec::new();
        let mut podcasts = Vec::new();
        let mut auto_playlists = None;
        match operation {
            OutboxOp::SetCursor { episode_id, .. } | OutboxOp::MarkPlayed { episode_id, .. } => {
                episodes.push(*episode_id);
            }
            OutboxOp::TriggerDownload { episode_ids }
            | OutboxOp::RemoveServerDownload { episode_ids } => {
                episodes.extend(episode_ids);
            }
            OutboxOp::AddToPlaylist { playlist_id, .. }
            | OutboxOp::RemoveFromPlaylist { playlist_id, .. }
            | OutboxOp::MoveInPlaylist { playlist_id, .. }
            | OutboxOp::ReorderPlaylist { playlist_id, .. }
            | OutboxOp::UpdatePlaylist { playlist_id, .. } => playlists.push(*playlist_id),
            OutboxOp::MovePlaylist { .. } => {
                playlists.extend(self.store.list_playlists().await?.iter().map(|row| row.id));
            }
            OutboxOp::Unsubscribe { podcast_id } | OutboxOp::RemovePodcastConfig { podcast_id } => {
                podcasts.push(*podcast_id)
            }
            OutboxOp::UpdatePodcastConfig { config_id, .. } => {
                podcasts.extend(
                    self.store
                        .list_podcasts()
                        .await?
                        .iter()
                        .filter(|row| row.podcast_config_id == Some(*config_id))
                        .map(|row| row.id),
                );
            }
            OutboxOp::SetPodcastAutoPlaylists { podcast_id, .. } => {
                auto_playlists = Some((
                    *podcast_id,
                    api.get_podcast_auto_playlists(*podcast_id).await?,
                ));
            }
            OutboxOp::Subscribe { .. } => {}
        }
        let mut changes = StoreChanges::default();
        for id in episodes {
            match api.get_episode(id, &[EpisodeInclude::Playback]).await {
                Ok(episode) => {
                    if let Some(playback) = episode.playback.clone() {
                        changes.playbacks.push(playback);
                    } else {
                        changes.deleted_playbacks.push(id);
                    }
                    changes.episodes.push(episode);
                }
                Err(halogen_apiclient::ApiError::Server {
                    status: 403 | 404, ..
                }) => {
                    changes.deleted_episodes.push(id);
                    changes.deleted_playbacks.push(id);
                }
                Err(error) => return Err(error.into()),
            }
        }
        for id in playlists {
            match api
                .get_playlist_including(id, &[PlaylistInclude::EpisodeIds])
                .await
            {
                Ok(playlist) => changes.playlists.push(playlist),
                Err(halogen_apiclient::ApiError::Server {
                    status: 403 | 404, ..
                }) => changes.deleted_playlists.push(id),
                Err(error) => return Err(error.into()),
            }
        }
        for id in podcasts {
            match api.get_podcast(id).await {
                Ok(podcast) => changes.podcasts.push(podcast),
                Err(halogen_apiclient::ApiError::Server {
                    status: 403 | 404, ..
                }) => changes.deleted_podcasts.push(id),
                Err(error) => return Err(error.into()),
            }
        }
        self.store.commit_changes(&changes, &[]).await?;
        for id in &changes.deleted_playbacks {
            self.playbacks.playbacks.remove(id);
        }
        for playback in &changes.playbacks {
            self.playbacks
                .playbacks
                .insert(playback.episode_id, playback.clone());
        }
        for id in &changes.deleted_episodes {
            self.app_state.episodes_by_id.remove(id);
            for rows in self.app_state.episodes_by_podcast.values_mut() {
                rows.retain(|row| row != id);
            }
        }
        for id in changes.deleted_playlists {
            self.remove_playlist_locally(id).await;
        }
        for id in changes.deleted_podcasts {
            self.remove_podcast_locally(id);
        }
        for row in &changes.podcasts {
            self.unsubscribed.remove(&row.id);
        }
        self.cache_episodes(changes.episodes).await;
        self.cache_playlists(changes.playlists).await;
        self.cache_podcasts(changes.podcasts).await;
        if let Some((id, rows)) = auto_playlists {
            self.podcasts
                .auto_playlist_add_to_start_by_podcast
                .insert(id, rows.first().and_then(|row| row.add_to_start));
            self.podcasts
                .auto_playlists_by_podcast
                .insert(id, rows.iter().map(|row| row.playlist_id).collect());
        }
        self.publish();
        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "rejection/tests.rs"]
mod tests;
