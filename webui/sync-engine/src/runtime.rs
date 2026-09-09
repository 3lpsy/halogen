//! Separate sync from rendering with WorkerEvent messages; the main-thread provider applies signals and toasts.
//! ToWorker/FromWorker serialize the protocol as JSON for the dedicated worker boundary.

use crate::Command;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use halogen_webui_app_state::{
    ConnectionState, DownloadState, EpisodeState, HistoryState, PlaybackState, PlaylistState,
    PodcastState, SessionState,
};
use halogen_webui_component_toast::{ToastDecision, ToastLevel};

/// One published state mirror (or toast) emitted by the worker for the main thread to apply. The worker sends these
/// instead of writing Dioxus signals directly. Serialized with serde so it can cross the Web Worker `postMessage`
/// boundary as a [`FromWorker::Event`] (the 8 state types it carries derive serde too).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WorkerEvent {
    Episode(EpisodeState),
    Podcasts(PodcastState),
    Playlists(PlaylistState),
    Playbacks(PlaybackState),
    History(HistoryState),
    Downloads(DownloadState),
    Connection(ConnectionState),
    Session(SessionState),
    Toast {
        level: ToastLevel,
        message: String,
        timeout_ms: Option<u32>,
    },
}

/// Main-thread → worker messages exchanged over `postMessage` as `serde_json` strings. The handshake: the main thread
/// creates the Worker and immediately posts [`ToWorker::Init`] carrying the active-user namespace `segment` (the worker
/// has its own globals and can't read the main thread's `namespace::segment()`). It then queues [`ToWorker::Command`]s
/// until the worker replies [`FromWorker::Ready`], and flushes the queue.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ToWorker {
    /// Open the worker's own stores under this user namespace segment and start the sync loop. Always the first
    /// message. `enabled`/`level` seed the worker's device-log capture from the main thread's current setting so worker
    /// logs are captured at the same threshold. (Live toggling after init is a documented follow-up, there's no
    /// live-update channel today; the worker reads these once here.)
    Init {
        segment: String,
        enabled: bool,
        level: halogen_webui_logging::Level,
    },
    /// A UI command to forward into the worker's command channel.
    Command(Box<Command>),
}

/// Worker → main-thread messages exchanged over `postMessage` as `serde_json` strings.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FromWorker {
    /// The worker has opened its stores and started its loop; the main thread may now
    /// flush queued commands.
    Ready,
    /// A published state mirror / toast for the main thread's applier to apply.
    Event(WorkerEvent),
    /// A device-log line captured in the worker, forwarded for the main thread to
    /// [`ingest`](halogen_webui_logging::ingest) into the shared ring + `halogen.logs`
    /// store. The worker has no log store of its own; the main thread is the single
    /// persister, so worker + UI logs share one `/logs/device` viewer + one store.
    Log(halogen_webui_logging::LogLine),
}

/// A toast emitter mirroring [`halogen_webui_component_toast::ToastHandle`]'s surface, but routed
/// through the worker's [`WorkerEvent`] channel instead of writing the toast signal
/// directly. The worker call sites are unchanged.
#[derive(Clone)]
pub struct WorkerToasts {
    tx: UnboundedSender<WorkerEvent>,
}

impl WorkerToasts {
    pub fn new(tx: UnboundedSender<WorkerEvent>) -> Self {
        Self { tx }
    }

    /// Raise a toast at the given level.
    pub fn show(&self, level: ToastLevel, message: impl Into<String>, timeout_ms: Option<u32>) {
        let _ = self.tx.unbounded_send(WorkerEvent::Toast {
            level,
            message: message.into(),
            timeout_ms,
        });
    }

    pub fn info(&self, message: impl Into<String>) {
        self.show(
            ToastLevel::Info,
            message,
            Some(ToastLevel::Info.default_timeout_ms()),
        );
    }

    pub fn success(&self, message: impl Into<String>) {
        self.show(
            ToastLevel::Success,
            message,
            Some(ToastLevel::Success.default_timeout_ms()),
        );
    }

    #[allow(dead_code)]
    pub fn warn(&self, message: impl Into<String>) {
        self.show(
            ToastLevel::Warning,
            message,
            Some(ToastLevel::Warning.default_timeout_ms()),
        );
    }

    pub fn error(&self, message: impl Into<String>) {
        self.show(
            ToastLevel::Error,
            message,
            Some(ToastLevel::Error.default_timeout_ms()),
        );
    }

    /// Apply a classified [`ToastDecision`]: a `Toast(..)` decision is shown;
    /// other decisions are handled by the caller. Returns the decision so callers
    /// can act on it (mirrors `ToastHandle::apply`).
    pub fn apply(&self, decision: ToastDecision) -> ToastDecision {
        if let ToastDecision::Toast(level, ref message) = decision {
            self.show(level, message.clone(), Some(level.default_timeout_ms()));
        }
        decision
    }
}

/// Formalizes the worker seam: the UI dispatches [`Command`]s through this sender,
/// which a concrete runtime (in-process today; native/web in Stage B) carries to the
/// worker. The trait stays lightweight — its purpose is to mark the boundary.
pub trait BackgroundRuntime {
    /// Sender the UI dispatches Commands through.
    fn commands(&self) -> UnboundedSender<Command>;
}

/// The worker's command + event channels, built together by [`worker_seam`]. The
/// caller spawns the worker on the local executor (draining `commands_rx`) and drains
/// `events_rx` on the main thread to apply [`WorkerEvent`]s.
pub struct WorkerSeam {
    pub commands_tx: UnboundedSender<Command>,
    pub commands_rx: UnboundedReceiver<Command>,
    pub events_tx: UnboundedSender<WorkerEvent>,
    pub events_rx: UnboundedReceiver<WorkerEvent>,
}

/// Build the worker's command + event channels (two unbounded channels). The provider
/// wires them; this keeps the channel construction in one place for Stage B.
pub fn worker_seam() -> WorkerSeam {
    let (commands_tx, commands_rx) = futures::channel::mpsc::unbounded();
    let (events_tx, events_rx) = futures::channel::mpsc::unbounded();
    WorkerSeam {
        commands_tx,
        commands_rx,
        events_tx,
        events_rx,
    }
}

#[cfg(test)]
mod tests {
    //! Protocol serde round-trip guards: every message that crosses the Web Worker
    //! `postMessage` boundary must survive `to_string` → `from_str` unchanged. These
    //! run natively (picked up by `just test-ui`) so a serde-breaking field change is
    //! caught without a browser.

    use super::*;
    use chrono::{TimeZone, Utc};
    use halogen_webui_app_state::{
        ClientDownloadState, ConnectionHealth, ConnectionState, DownloadState, EpisodeState,
        HistoryState, PlaybackState, PlaylistState, PodcastState, QueueState, SessionState,
        SyncPhase, SyncStatus,
    };
    use halogen_webui_commands::RedactedToken;
    use halogen_webui_component_toast::ToastLevel;
    use halogen_wire::{
        DownloadStatus, EpisodeData, PlaybackData, PlaybackStatus, PlaylistData, PodcastData,
    };

    /// A fixed timestamp so fixtures are deterministic.
    fn ts() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()
    }

    fn ep(id: i32) -> EpisodeData {
        EpisodeData {
            id,
            podcast_id: 7,
            title: format!("Episode {id}"),
            description: Some("desc".into()),
            content_url: "https://example.com/ep.mp3".into(),
            guid: Some("guid".into()),
            art_url: None,
            published_at: Some(ts()),
            downloaded_at: None,
            content_file_path: None,
            download_size: Some(1234),
            art_file_path: None,
            download_status: DownloadStatus::Downloaded,
            download_started_at: None,
            download_attempts: 1,
            playback_status: PlaybackStatus::Unplayed,
            duration_secs: Some(600),
            created_at: ts(),
            updated_at: ts(),
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    fn pod(id: i32) -> PodcastData {
        PodcastData {
            id,
            title: format!("Podcast {id}"),
            description: "desc".into(),
            feed_url: "https://example.com/feed".into(),
            art_url: None,
            author: Some("author".into()),
            polled_at: Some(ts()),
            podcast_config_id: None,
            art_file_path: None,
            etag: None,
            last_modified: None,
            podcast_config: None,
            created_at: ts(),
            updated_at: ts(),
            episode_count: Some(3),
            feed_url_redirects: None,
        }
    }

    fn playlist(id: i32) -> PlaylistData {
        PlaylistData {
            id,
            name: format!("Playlist {id}"),
            description: None,
            is_default: true,
            position: 0,
            on_remove_delete_file_server: false,
            on_remove_delete_file_client: false,
            created_at: ts(),
            updated_at: ts(),
            episode_ids: Some(vec![1, 2, 3]),
            episode_playlist: None,
        }
    }

    fn playback(episode_id: i32) -> PlaybackData {
        PlaybackData {
            id: 1,
            user_id: 1,
            episode_id,
            cursor: 42,
            completed: false,
            created_at: ts(),
            updated_at: ts(),
        }
    }

    /// Assert `value` survives a `to_string` → `from_str` round-trip unchanged.
    fn round_trip<T>(value: &T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back, "round-trip mismatch for {json}");
    }

    /// A non-trivial representative for every [`WorkerEvent`] variant.
    fn sample_events() -> Vec<WorkerEvent> {
        let mut episode = EpisodeState::default();
        episode.episodes_by_id.insert(1, ep(1));
        episode.episodes_by_podcast.insert(7, vec![1]);

        let mut podcasts = PodcastState::default();
        podcasts.podcasts_by_id.insert(7, pod(7));
        podcasts.auto_playlists_by_podcast.insert(7, vec![5, 9]);

        let mut playlists = PlaylistState::default();
        playlists.playlists.push(playlist(5));
        playlists.episodes_by_playlist.insert(5, vec![1, 2, 3]);
        playlists.queue = QueueState::Present(5);

        let mut playbacks = PlaybackState::default();
        playbacks.playbacks.insert(1, playback(1));

        let mut downloads = DownloadState::default();
        downloads
            .client_downloads
            .insert(1, ClientDownloadState::Downloading);
        downloads.download_progress.insert(1, 55);
        downloads.server_download_progress.insert(2, 10);

        let connection = ConnectionState {
            sync_status: SyncStatus::Syncing {
                phase: SyncPhase::Pull,
            },
            connection: ConnectionHealth::Degraded { rtt_ms: 250 },
            last_error: Some("boom".into()),
            last_synced_at: Some(ts()),
        };

        // The cold-start defaults (SyncStatus::Unknown / ConnectionHealth::Unknown)
        // cross the postMessage boundary too — pin their wire forms.
        let boot_connection = ConnectionState::default();

        vec![
            WorkerEvent::Episode(episode),
            WorkerEvent::Podcasts(podcasts),
            WorkerEvent::Playlists(playlists),
            WorkerEvent::Playbacks(playbacks),
            WorkerEvent::History(HistoryState {
                history_has_more: false,
                history_next_page: 4,
            }),
            WorkerEvent::Downloads(downloads),
            WorkerEvent::Connection(connection),
            WorkerEvent::Connection(boot_connection),
            WorkerEvent::Session(SessionState { auth_expired: true }),
            WorkerEvent::Toast {
                level: ToastLevel::Error,
                message: "something failed".into(),
                timeout_ms: Some(8000),
            },
        ]
    }

    fn sample_command() -> Command {
        Command::SetAuth {
            server_url: "https://srv.example".into(),
            token: RedactedToken("jwt-token".into()),
        }
    }

    #[test]
    fn worker_event_variants_round_trip() {
        for ev in sample_events() {
            round_trip(&ev);
        }
    }

    #[test]
    fn command_round_trips() {
        round_trip(&sample_command());
    }

    #[test]
    fn to_worker_round_trips() {
        round_trip(&ToWorker::Init {
            segment: "u42".into(),
            enabled: true,
            level: halogen_webui_logging::Level::Debug,
        });
        let command = sample_command();
        let message = ToWorker::Command(Box::new(command.clone()));
        round_trip(&message);
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            serde_json::json!({ "Command": command })
        );
    }

    #[test]
    fn from_worker_round_trips() {
        round_trip(&FromWorker::Ready);
        for ev in sample_events() {
            round_trip(&FromWorker::Event(ev));
        }
    }

    /// A forwarded worker device-log line must survive the `postMessage` round-trip
    /// so the main thread can `ingest` it into the shared ring/store unchanged.
    #[test]
    fn from_worker_log_round_trips() {
        round_trip(&FromWorker::Log(halogen_webui_logging::LogLine {
            ts_ms: 1_700_000_000_123,
            level: halogen_webui_logging::Level::Warn,
            target: "halogen_webui_sync_engine::runtime".into(),
            msg: "worker log line key=value".into(),
        }));
    }
}
