//! App-wide context providers, one component per concern, composed at the root.
//!
//! Every provider calls its hooks (`use_signal`/`use_coroutine`/`use_context_provider`)
//! **unconditionally**, so the rules of hooks always hold (the previous inline
//! `if … { use_context_provider }` block in `main.rs` violated this). Gating is
//! data-driven: the worker idles until auth arrives; routing is guarded.

mod accounts;
mod app;
mod config;
mod connection;
mod discover;
mod downloads;
mod episode;
mod history;
mod playback;
mod player;
mod playlist;
mod podcast;
mod session;
mod toast;
mod webview_media;
mod worker;

pub use accounts::AccountsProvider;
pub use app::{AppProviders, LoadingSplash};
pub use config::ConfigProvider;
pub use connection::ConnectionStateProvider;
pub use discover::DiscoverStateProvider;
pub use downloads::DownloadStateProvider;
pub use episode::EpisodeStateProvider;
pub use history::HistoryStateProvider;
pub use playback::PlaybackStateProvider;
pub use player::PlayerProvider;
pub use playlist::PlaylistStateProvider;
pub use podcast::PodcastStateProvider;
pub use session::SessionStateProvider;
pub use toast::ToastProvider;
pub use webview_media::WebviewMediaBridge;
pub use worker::WorkerProvider;
