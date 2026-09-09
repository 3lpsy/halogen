use std::rc::Rc;

use futures::{StreamExt, channel::mpsc};
use halogen_sync_journal::BufferedStore;
use halogen_webui_app_state::{
    ConnectionState, DownloadState, EpisodeState, HistoryState, PlaybackState, PlaylistState,
    PodcastState, SessionState,
};
use halogen_webui_logging::error;

use crate::{Command, SyncService, WorkerToasts, tracked::Tracked};

/// Cache changes and their delivery intent become visible only after durable commit.
impl SyncService {
    pub(crate) async fn handle_command(&mut self, command: Command) {
        if !is_journaled(&command) {
            self.apply_command(command).await;
            return;
        }
        let snapshot = Snapshot::capture(self);
        let durable = self.store.clone();
        let buffered = Rc::new(BufferedStore::new(durable.clone()));
        self.store = buffered.clone();
        self.deferred_audio_removals = Some(Vec::new());
        self.deferred_server_polls = Some(Vec::new());
        self.deferred_device_starts = Some(Vec::new());
        let (toast_tx, mut toast_rx) = mpsc::unbounded();
        let toasts = std::mem::replace(&mut self.toasts, WorkerToasts::new(toast_tx));

        self.apply_command(command).await;
        let result = buffered.commit().await;
        self.store = durable;
        self.toasts = toasts;
        let removals = self.deferred_audio_removals.take().unwrap_or_default();
        let polls = self.deferred_server_polls.take().unwrap_or_default();
        let starts = self.deferred_device_starts.take().unwrap_or_default();
        match result {
            Ok(()) => {
                for episode_id in removals {
                    self.remove_device_audio(episode_id).await;
                }
                for episode_id in polls {
                    self.ensure_server_progress_poll(episode_id);
                }
                for (episode_id, server_ready) in starts {
                    self.spawn_device_transfer(episode_id, server_ready);
                }
                while let Some(event) = toast_rx.next().await {
                    let _ = self.events.unbounded_send(event);
                }
                self.publish();
            }
            Err(error) => {
                snapshot.restore(self);
                error!(%error, "Could not commit local mutation and journal");
                self.connection.last_error = Some(format!("Could not save action: {error}"));
                self.toasts
                    .error("Couldn't save your action on this device. Please try again.");
                self.publish();
            }
        }
    }
}

fn is_journaled(command: &Command) -> bool {
    matches!(
        command,
        Command::DownloadToDevice { .. }
            | Command::RedownloadDevice { .. }
            | Command::Subscribe { .. }
            | Command::Unsubscribe { .. }
            | Command::DownloadOnServer { .. }
            | Command::RemoveServerDownload { .. }
            | Command::RedownloadOnServer { .. }
            | Command::MarkPlayed { .. }
            | Command::SetCursor { .. }
            | Command::AddToPlaylist { .. }
            | Command::RemoveFromPlaylist { .. }
            | Command::MoveInPlaylist { .. }
            | Command::MovePlaylist { .. }
            | Command::ReorderPlaylist { .. }
            | Command::UpdatePlaylist { .. }
            | Command::UpdatePodcastConfig { .. }
            | Command::RemovePodcastConfig { .. }
            | Command::SetPodcastAutoPlaylists { .. }
    )
}

struct Snapshot {
    episodes: Tracked<EpisodeState>,
    podcasts: Tracked<PodcastState>,
    playlists: Tracked<PlaylistState>,
    playbacks: Tracked<PlaybackState>,
    history: Tracked<HistoryState>,
    downloads: Tracked<DownloadState>,
    connection: Tracked<ConnectionState>,
    session: Tracked<SessionState>,
    unsubscribed: std::collections::HashSet<i32>,
}

impl Snapshot {
    fn capture(service: &SyncService) -> Self {
        Self {
            episodes: service.app_state.clone(),
            podcasts: service.podcasts.clone(),
            playlists: service.playlists.clone(),
            playbacks: service.playbacks.clone(),
            history: service.history.clone(),
            downloads: service.downloads.clone(),
            connection: service.connection.clone(),
            session: service.session.clone(),
            unsubscribed: service.unsubscribed.clone(),
        }
    }

    fn restore(self, service: &mut SyncService) {
        service.app_state = self.episodes;
        service.podcasts = self.podcasts;
        service.playlists = self.playlists;
        service.playbacks = self.playbacks;
        service.history = self.history;
        service.downloads = self.downloads;
        service.connection = self.connection;
        service.session = self.session;
        service.unsubscribed = self.unsubscribed;
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
