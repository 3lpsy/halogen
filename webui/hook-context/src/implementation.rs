//! Thin typed `use_context` accessors, the single way pages/components reach shared context state. Each is a one-liner
//! over `use_context::<T>()`; they live together here rather than in a file apiece. Hooks with real logic (memo slices,
//! derived state) stay in their own modules (`use_episodes`, `use_is_admin`, …).

use dioxus::prelude::*;

use halogen_webui_accounts::accounts::Accounts;
use halogen_webui_app_state::{
    ConnectionState, DiscoverState, DownloadState, HistoryState, PlaybackState, PlaylistState,
    PodcastState, SessionState,
};
use halogen_webui_commands::Command;
use halogen_webui_component_toast::{ToastHandle, ToastQueue};
use halogen_webui_config::ClientConfig;
use halogen_webui_player::{NowPlaying, PlayContext, PlayerController, PlayingIdentity};
use halogen_webui_store::StoreHandle;

// Re-exported so components raising toasts can pick a level/timeout without a
// direct `halogen-webui-component-toast` dep (e.g. the episode-row swipe's brief info toast).
pub use halogen_webui_component_toast::ToastLevel;

/// Read the app-wide client config signal. The mutate-and-save *action* lives in
/// [`config_actions::persist_config`](halogen_webui_config::config_actions::persist_config), not here.
pub fn use_config() -> Signal<ClientConfig> {
    use_context::<Signal<ClientConfig>>()
}

/// The worker dispatch handle. Pages/components send `Command`s through this
/// (usually via the typed `commands::*` helpers, not directly).
pub fn use_dispatch() -> Coroutine<Command> {
    use_context::<Coroutine<Command>>()
}

/// The shared local-store handle (read access for components). Server-paged views
/// read their cached page through this; writes still go via `use_dispatch` so the
/// worker stays the single store writer. `StoreHandle(None)` when the store failed
/// to open.
pub fn use_store() -> StoreHandle {
    use_context::<StoreHandle>()
}

/// Read/write the ephemeral Discover search store (last query + results).
///
/// Written by the `/discover` page on each successful search; read by the
/// `/discover/:id` detail page to resolve a clicked result.
pub fn use_discover_store() -> Signal<DiscoverState> {
    use_context::<Signal<DiscoverState>>()
}

/// Read/write handle to the device-global account registry (provided by
/// `AccountsProvider`). Mutating `active_user_id` triggers the keyed remount that
/// hot-swaps the active user. The account *actions* (switch / sign out / wipe)
/// live in [`halogen_webui_accounts::account_actions`].
pub fn use_accounts() -> Signal<Accounts> {
    use_context::<Signal<Accounts>>()
}

/// The shared toast queue signal.
pub fn use_toasts() -> Signal<ToastQueue> {
    use_context::<Signal<ToastQueue>>()
}

/// A `Copy` [`ToastHandle`] for raising toasts from event handlers / spawns.
pub fn use_toast() -> ToastHandle {
    ToastHandle::new(use_toasts())
}

/// The shared player controller (provided by `PlayerProvider`).
pub fn use_player_controller() -> Signal<PlayerController> {
    use_context::<Signal<PlayerController>>()
}

/// The current player state signal (provided by `PlayerProvider`). Lives outside `EpisodeState` so player ticks don't
/// re-render the rest of the app. Reading this subscribes to `position_secs` too, so it re-runs ~4×/sec while playing,
/// fine for the player UI (it shows the position), but the episode rows must NOT use it (one row × 30 × 4/sec
/// recomputes). Rows use the gated identity projection below instead.
pub fn use_now_playing() -> Signal<Option<NowPlaying>> {
    use_context::<Signal<Option<NowPlaying>>>()
}

/// The play-context signal (provided by `PlayerProvider`): the playlist the user pressed play from (`PlayContext(None)`
/// = queue semantics). Written by the controller's `*_in` play entries; read by the "up next" surfaces (the full-screen
/// player preview, the list next-up marker, the transport buttons) so they follow the playlist being listened through.
pub fn use_play_context() -> Signal<PlayContext> {
    use_context::<Signal<PlayContext>>()
}

/// The position-free `now_playing` projection (provided by `PlayerProvider`): a `PartialEq`-gated [`Memo`] of
/// `(episode_id, state)`. Episode rows read THIS to derive current/playing/preparing without re-running on every
/// ~4×/sec position tick, the projection is unchanged by a tick, so its memo doesn't invalidate the rows that depend on
/// it.
pub fn use_now_playing_identity() -> Memo<Option<PlayingIdentity>> {
    use_context::<Memo<Option<PlayingIdentity>>>()
}

/// The shared device/server download-state signal (provided by
/// `DownloadStateProvider`). A separate signal from `EpisodeState`, so a download-progress
/// byte re-renders only the row badges + Downloads list, not every `EpisodeState` reader.
pub fn use_downloads() -> Signal<DownloadState> {
    use_context::<Signal<DownloadState>>()
}

/// Read the whole connectivity/sync signal independently of episode data. Prefer equality-gated use_sync_status,
/// use_is_offline, or use_connection_health memos for narrow consumers.
pub fn use_connection() -> Signal<ConnectionState> {
    use_context::<Signal<ConnectionState>>()
}

/// The shared podcast-pool signal (provided by `PodcastStateProvider`), i.e. the cached podcasts + per-podcast
/// auto-playlist config. A separate signal from `EpisodeState`, so a podcast cache write or auto-playlist edit
/// re-renders only podcast consumers (the podcasts list/detail, the row podcast-title lookup), not every `EpisodeState`
/// reader.
pub fn use_podcasts() -> Signal<PodcastState> {
    use_context::<Signal<PodcastState>>()
}

/// The shared History paging-cursor signal (provided by `HistoryStateProvider`).
/// A separate signal from `EpisodeState`, so advancing the cursor as History pages
/// re-renders only the History page-ahead effect, not every `EpisodeState` reader.
pub fn use_history() -> Signal<HistoryState> {
    use_context::<Signal<HistoryState>>()
}

/// The shared playback-cursors signal (provided by `PlaybackStateProvider`). A separate
/// signal from `EpisodeState`, so a cursor save (seek / mark-played) or a History page
/// re-renders only playback consumers (row progress bars + played markers, the History
/// list), not every `EpisodeState` reader.
pub fn use_playbacks() -> Signal<PlaybackState> {
    use_context::<Signal<PlaybackState>>()
}

/// The shared playlist + queue signal (provided by `PlaylistStateProvider`). Split
/// out of `EpisodeState` so a playlist mutation (add/remove/reorder, default resolution)
/// only re-renders playlist/queue consumers (the playlists list/detail, the queue
/// page, the row "up next" markers), not every `EpisodeState` reader.
pub fn use_playlists() -> Signal<PlaylistState> {
    use_context::<Signal<PlaylistState>>()
}

/// The shared session-liveness signal (provided by `SessionStateProvider`), i.e.
/// the worker-owned `auth_expired` bit. `WorkerProvider` watches it to sign out on
/// a 401; exposed here for symmetry with the other worker-owned slices.
pub fn use_session() -> Signal<SessionState> {
    use_context::<Signal<SessionState>>()
}
