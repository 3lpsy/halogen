//! `halogen-webui-app-state`, the reactive application-state data layer. `EpisodeState` (the canonical snapshot the
//! sync worker owns and every UI subscriber reads via signal) plus the per-domain state slices (podcast / playlist /
//! playback / history / download / session / connection), the sync/connectivity/queue status enums, the media/art URL
//! builders, and the ephemeral `DiscoverState` search results. Pure data types; a leaf of the UI crate graph.

pub mod connection_state;
pub mod discover;
pub mod download_state;
pub mod history_state;
pub mod media_url;
pub mod playback_state;
pub mod playlist_state;
pub mod podcast_state;
pub mod session_state;
pub mod state;

pub use connection_state::ConnectionState;
pub use discover::{DiscoverMode, DiscoverState};
pub use download_state::DownloadState;
pub use history_state::HistoryState;
pub use playback_state::PlaybackState;
pub use playlist_state::{PlaylistState, QueueState};
pub use podcast_state::PodcastState;
pub use session_state::SessionState;
pub use state::{ClientDownloadState, ConnectionHealth, EpisodeState, SyncPhase, SyncStatus};
