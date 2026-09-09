use dioxus::prelude::*;

use halogen_webui_app_state::{ConnectionHealth, EpisodeState, SyncStatus};

use halogen_webui_hook_context::use_connection;

/// Read worker-owned episode state; podcast, playlist, playback, and history have sibling signals. Every read
/// subscribes to each episode publication, so narrow consumers should use a PartialEq memo such as [`use_sync_status`]
/// to suppress unchanged renders.
pub fn use_episodes() -> Signal<EpisodeState> {
    use_context::<Signal<EpisodeState>>()
}

/// The current sync status, as a `PartialEq`-gated memo slice. The slice exemplar: the memo subscribes to the whole
/// `ConnectionState` signal (it re-runs on every connection publish), but its subscribers, the navbar pill, only
/// re-render when `SyncStatus` itself changes. Reads the dedicated `ConnectionState` signal now, so it no longer wakes
/// on unrelated `EpisodeState` publishes (cache writes, download progress) at all.
pub fn use_sync_status() -> Memo<SyncStatus> {
    let conn = use_connection();
    use_memo(move || conn.read().sync_status.clone())
}

/// Whether the worker is offline, as a `PartialEq`-gated memo slice. The common case of [`use_sync_status`], most
/// consumers only care about the offline/online bit to disable a control or swap an offline hint, not the full
/// `Syncing` phase. Reading it (`use_is_offline()()`) subscribes the component only to flips of the offline bit. For a
/// non-reactive read inside a submit closure, prefer `connection.peek().is_offline()` instead.
pub fn use_is_offline() -> Memo<bool> {
    let conn = use_connection();
    use_memo(move || conn.read().is_offline())
}

/// The live connection health (green/yellow/red source), as a `PartialEq`-gated
/// memo slice — the navbar status dot re-renders only when the tier (or smoothed
/// RTT) actually changes. See [`use_sync_status`] for why a memo slice (not
/// `use_connection()().connection`) is the right tool here.
pub fn use_connection_health() -> Memo<ConnectionHealth> {
    let conn = use_connection();
    use_memo(move || conn.read().connection)
}
