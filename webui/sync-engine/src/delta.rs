use crate::SyncService;
use halogen_webui_store::StoreChanges;

impl SyncService {
    /// Publish only committed server changes; device media and in-flight downloads stay independent.
    pub(crate) fn apply_server_changes(&mut self, changes: StoreChanges) {
        if changes.reset_cache {
            self.app_state.episodes_by_id.clear();
            self.app_state.episodes_by_podcast.clear();
            self.podcasts.podcasts_by_id.clear();
            self.podcasts.auto_playlists_by_podcast.clear();
            self.podcasts.auto_playlist_add_to_start_by_podcast.clear();
            self.playlists.playlists.clear();
            self.playlists.episodes_by_playlist.clear();
            self.playbacks.playbacks.clear();
            self.unsubscribed.clear();
        }
        for id in changes.deleted_podcasts {
            self.podcasts.podcasts_by_id.remove(&id);
            self.app_state.episodes_by_podcast.remove(&id);
        }
        for id in changes.deleted_episodes {
            self.app_state.episodes_by_id.remove(&id);
            for rows in self.app_state.episodes_by_podcast.values_mut() {
                rows.retain(|row| *row != id);
            }
            self.downloads.server_download_progress.remove(&id);
        }
        for id in changes.deleted_playlists {
            self.playlists.playlists.retain(|row| row.id != id);
            self.playlists.episodes_by_playlist.remove(&id);
        }
        for id in changes.deleted_playbacks {
            self.playbacks.playbacks.remove(&id);
        }
        for id in changes.deleted_auto_playlists {
            self.podcasts.auto_playlists_by_podcast.remove(&id);
            self.podcasts
                .auto_playlist_add_to_start_by_podcast
                .remove(&id);
        }
        for row in changes.podcasts {
            self.unsubscribed.remove(&row.id);
            self.podcasts.podcasts_by_id.insert(row.id, row);
        }
        for row in changes.episodes {
            let ids = self
                .app_state
                .episodes_by_podcast
                .entry(row.podcast_id)
                .or_default();
            if !ids.contains(&row.id) {
                ids.push(row.id);
            }
            if row.download_status != halogen_wire::DownloadStatus::Downloading {
                self.downloads.server_download_progress.remove(&row.id);
            }
            self.app_state.episodes_by_id.insert(row.id, row);
        }
        for row in changes.playlists {
            if let Some(ids) = &row.episode_ids {
                self.playlists
                    .episodes_by_playlist
                    .insert(row.id, ids.clone());
            }
            self.playlists.playlists.retain(|old| old.id != row.id);
            self.playlists.playlists.push(row);
        }
        self.playlists
            .playlists
            .sort_by_key(|row| (row.position, row.id));
        for row in changes.playbacks {
            self.playbacks.playbacks.insert(row.episode_id, row);
        }
        for (id, rows) in changes.auto_playlists {
            self.podcasts
                .auto_playlist_add_to_start_by_podcast
                .insert(id, rows.first().and_then(|row| row.add_to_start));
            self.podcasts
                .auto_playlists_by_podcast
                .insert(id, rows.into_iter().map(|row| row.playlist_id).collect());
        }
        self.playlists.recompute_queue(true);
        self.publish();
    }
}
