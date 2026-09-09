use halogen_wire::{
    EpisodeData, OrderDirection, PlaylistData, PlaylistReorderField, PlaylistUpdateData,
    PodcastConfigUpdateData, PodcastData,
};

/// A bearer token whose `Debug` prints `"***"`, never the secret. `Command` derives `Debug` and the worker logs
/// `debug!(command = ?cmd)`, which, with device logging enabled, persists to the on-disk/IndexedDB log ring. Wrapping
/// the token makes the redaction structural: no call site, and no future `Command` variant, can leak the JWT into the
/// logs by accident.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RedactedToken(pub String);

impl std::fmt::Debug for RedactedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"***\"")
    }
}

/// Commands the UI sends to the sync worker.
///
/// The worker is the single writer of canonical data; the UI reads
/// reactive signals and sends commands. Commands are fire-and-forget.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Command {
    /// Subscribe to a podcast feed, carrying any directory metadata already known
    /// (Discover results) so the created row has a real title immediately.
    Subscribe {
        feed_url: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        author: Option<String>,
    },
    /// Unsubscribe from a podcast.
    Unsubscribe { podcast_id: i32 },
    /// Mark requested IDs Downloading, ensure server copies, then fetch bytes to the device and report completion. Skip
    /// existing/in-flight IDs; one row uses spinner feedback, batches get summary feedback, and offline batches toast
    /// once.
    DownloadToDevice { episode_ids: Vec<i32> },
    /// Remove episodes' device downloads. One id is quiet; many summary-toast.
    RemoveDownload { episode_ids: Vec<i32> },
    /// Trigger a server-side download only (no device download). One id animates
    /// the per-row progress ring and queues a single op; many queue ONE bulk op
    /// and summary-toast.
    DownloadOnServer { episode_ids: Vec<i32> },
    /// Remove the server's downloaded copies. One id queues a single op quietly;
    /// many queue ONE bulk op and summary-toast. Drops the device copy too —
    /// removing from the server removes it locally as well.
    RemoveServerDownload { episode_ids: Vec<i32> },
    /// Force-fresh device redownload — per id remove the local copy, then re-pull
    /// (device download dedupes, so the remove is what forces a refetch).
    RedownloadDevice { episode_ids: Vec<i32> },
    /// Force-fresh server redownload — remove then download (two bulk API calls,
    /// drained in order; the server remove lands before the trigger).
    RedownloadOnServer { episode_ids: Vec<i32> },
    /// Mark an episode as played / unplayed.
    MarkPlayed { episode_id: i32, played: bool },
    /// Set the playback cursor position (debounced by UI before sending).
    SetCursor { episode_id: i32, cursor: i64 },
    /// Add episodes to a playlist (e.g. the queue). One id (a row action) adds
    /// quietly and honors the front-of-queue preference when the target is the
    /// queue; multiple (multiselect) append and get a summary toast. Optimistic
    /// local insert, then ONE bulk API call per drain.
    AddToPlaylist {
        playlist_id: i32,
        episode_ids: Vec<i32>,
    },
    /// Remove episodes from a playlist. One id is quiet; many summary-toast.
    /// Optimistic local removal, then ONE bulk API call per drain.
    RemoveFromPlaylist {
        playlist_id: i32,
        episode_ids: Vec<i32>,
    },
    /// Reorder an episode within a playlist to target index `to` (manual order).
    MoveInPlaylist {
        playlist_id: i32,
        episode_id: i32,
        to: i32,
    },
    /// Reorder a playlist within the user's manual order to target index `to`
    /// (offline-capable): optimistic local reorder + a `MovePlaylist` outbox op.
    MovePlaylist { playlist_id: i32, to: i32 },
    /// Smart-reorder a playlist's episodes by `field`/`direction`, baking the order
    /// into the `Custom` (position) sequence (offline-capable): optimistic local
    /// reorder + a `ReorderPlaylist` outbox op (the server is authoritative).
    ReorderPlaylist {
        playlist_id: i32,
        field: PlaylistReorderField,
        direction: OrderDirection,
    },
    /// Persist a freshly-fetched page of episodes (server-paged views like
    /// `/latest` fetch their own pages, then hand them back so the worker — the
    /// single store writer — upserts them to the cache and merges them into
    /// `episodes_by_id` for O(1) lookups (player, episode detail).
    CacheEpisodes { episodes: Vec<EpisodeData> },
    /// Persist a freshly-fetched page of podcasts (the paged podcasts list fetches
    /// its own pages, then hands them back so the worker upserts them to the cache
    /// and merges them into `podcasts_by_id` for O(1) detail/title lookups).
    CachePodcasts { podcasts: Vec<PodcastData> },
    /// Edit a playlist OFFLINE: apply optimistically to local state and queue an
    /// `UpdatePlaylist` outbox op. Online edits don't use this — they go direct so
    /// the form can surface server errors.
    UpdatePlaylist {
        playlist_id: i32,
        data: PlaylistUpdateData,
    },
    /// Upsert playlists into local state + store after a DIRECT create/update
    /// (which bypasses the outbox to get a real server id). Makes navigation to the
    /// new playlist instant; a following `RefreshNow` reconciles the full set.
    CachePlaylists { playlists: Vec<PlaylistData> },
    /// Delete a playlist server-side. Playlist CRUD is ONLINE-ONLY (like the
    /// direct create/edit forms), so this is a direct API call, not an outbox op:
    /// on success the playlist is dropped from local state + store; on failure the
    /// worker just toasts and nothing is queued.
    DeletePlaylist { playlist_id: i32 },
    /// Edit a podcast config OFFLINE: apply optimistically to the cached podcast and
    /// queue an `UpdatePodcastConfig` outbox op. Online edits go direct instead so
    /// the form can surface server errors.
    UpdatePodcastConfig {
        podcast_id: i32,
        config_id: i32,
        data: PodcastConfigUpdateData,
    },
    /// Remove a podcast config (offline-capable): optimistically unlink it from the
    /// cached podcast and queue a `RemovePodcastConfig` outbox op.
    RemovePodcastConfig { podcast_id: i32, config_id: i32 },
    /// Set the playlists a podcast auto-adds new episodes to (offline-capable): optimistically cache the set and queue
    /// a `SetPodcastAutoPlaylists` outbox op. Online edits go direct, then dispatch `CacheAutoPlaylists`.
    /// `add_to_start` is the podcast's insert-position override (`Some(true)` = start of the playlists, `Some(false)` =
    /// end, `None` = server default).
    SetPodcastAutoPlaylists {
        podcast_id: i32,
        playlist_ids: Vec<i32>,
        add_to_start: Option<bool>,
    },
    /// Cache a podcast's auto-playlist set after a DIRECT (online) fetch/save —
    /// pure state write, no outbox.
    CacheAutoPlaylists {
        podcast_id: i32,
        playlist_ids: Vec<i32>,
        add_to_start: Option<bool>,
    },
    /// Lazily load a podcast into `podcasts_by_id` for title/art lookups —
    /// episode lists no longer carry a nested podcast join, so rows ask for the
    /// parent on demand. De-duped by the worker (skips when already cached or a
    /// fetch is in flight), so it's safe to send from every visible row.
    EnsurePodcast { podcast_id: i32 },
    /// INTERNAL — sent by a spawned device-download task back into the worker
    /// when its byte fetch finishes. Carries only the outcome: on success the
    /// task has ALREADY written the bytes to the media store (routing megabytes
    /// through this Debug-logged enum would double peak memory).
    DeviceFetchComplete {
        episode_id: i32,
        result: Result<(), String>,
    },
    /// INTERNAL — a device-download task reporting incremental progress (percent
    /// 0–100) as chunks land, so the badge can render a filling ring. Throttled to
    /// integer-percent changes by the task.
    DeviceFetchProgress { episode_id: i32, percent: u8 },
    /// INTERNAL, a server-download poll task reporting the SERVER's fetch progress (percent 0–100) for an episode whose
    /// copy the server is downloading, so the download controls render a filling ring like the device flow. Polled ≈1
    /// Hz off the server's progress tracker; applied only while the episode's `download_status` is still `Downloading`.
    /// Throttled to integer-percent changes by the task.
    ServerDownloadProgress { episode_id: i32, percent: u8 },
    /// INTERNAL — a server-download poll task reporting that the server's fetch
    /// finished (or the poll budget ran out). Clears the in-flight ring + dedup
    /// guard and, when the refreshed body is carried, updates the durable
    /// `download_status`.
    ServerDownloadComplete {
        episode_id: i32,
        episode: Option<EpisodeData>,
    },
    /// INTERNAL — a lazily-fetched podcast (from [`Command::EnsurePodcast`])
    /// reported back by its spawned task. `None` = fetch failed; either way the
    /// in-flight guard is cleared so it can be retried.
    PodcastFetched {
        podcast_id: i32,
        podcast: Option<PodcastData>,
    },
    /// INTERNAL, a batch of lazily-fetched podcasts reported back by the pool-prime task (one
    /// `list_podcasts(filter.ids=…)` collapsing the per-row `EnsurePodcast` N+1). `podcast_ids` is the requested set
    /// (so every in-flight guard is cleared even for ids the server omitted); `podcasts` is `None` on fetch failure.
    PodcastsFetched {
        podcast_ids: Vec<i32>,
        podcasts: Option<Vec<PodcastData>>,
    },
    /// Lazily load an episode's chapter markers into `episodes_by_id` for the
    /// now-playing display. Chapters are an opt-in include the lists don't carry,
    /// so the expanded player asks for them on demand. De-duped by the worker
    /// (skips when already loaded or a fetch is in flight).
    EnsureEpisodeChapters { episode_id: i32 },
    /// INTERNAL — a lazily-fetched episode (from [`Command::EnsureEpisodeChapters`])
    /// reported back by its spawned task, carrying the full body with chapters.
    /// `None` = fetch failed; either way the in-flight guard is cleared.
    EpisodeChaptersFetched {
        episode_id: i32,
        episode: Option<EpisodeData>,
    },
    /// Resolve the queue (default playlist) via the targeted `/playlists/default`
    /// endpoint and update `PlaylistState.queue`. Cheap, lazy — dispatched when the queue
    /// is `Unknown` (e.g. the /queue page mounts) so reactivity has its target even
    /// if the default sits outside a paged playlist list. De-duped server-side cost.
    EnsureDefaultPlaylist,
    /// Page the History list's source: fetch the next `GET /playbacks` page
    /// (updated_at desc) and merge it into the `playbacks` overlay + local store.
    /// `reset` restarts paging from page 0 (mount / pull-to-refresh); otherwise it
    /// advances `history_next_page`. No-op when the server has no more pages.
    LoadHistory { reset: bool },
    /// Toggle manual "Go Offline" mode. `true` forces the worker offline — it skips
    /// all network sync (pull / drain / history) and reports `Offline` regardless of
    /// real connectivity; `false` reconnects with an immediate pull + drain.
    /// Persisted in `ClientConfig`; dispatched on the navbar toggle and on boot.
    SetOffline(bool),
    /// Mirror the "add to front of queue" preference into the worker. When `true`,
    /// adding an episode to the queue (default playlist) inserts at position 0
    /// instead of appending. Persisted in `ClientConfig.playback_prefs`; dispatched
    /// on boot and whenever the setting changes (like [`Command::SetOffline`]).
    SetAddToQueueFront(bool),
    /// Mirror the device-download preferences into the worker: `chunk_bytes` is the per-request chunk size (`None` = no
    /// chunking, fetch the whole file in one request) and `parallelism` is how many chunks are fetched concurrently
    /// within a single download. Persisted in `ClientConfig.download_prefs`; dispatched on boot and whenever the
    /// setting changes (like [`Command::SetOffline`]).
    SetDownloadPrefs {
        chunk_bytes: Option<u64>,
        parallelism: u8,
    },
    /// Force an immediate full pull from the server.
    RefreshNow,
    /// Update auth credentials; rebuilds the ApiClient. The token is wrapped in
    /// [`RedactedToken`] so it can't leak through the worker's `Debug` command log.
    SetAuth {
        server_url: String,
        token: RedactedToken,
    },
    /// Remove ALL locally-cached data for one episode, cached metadata, playback position/history, the
    /// device-downloaded audio blob, and its membership in local playlists/queue, WITHOUT touching the server. A local
    /// cache prune for recovery, NOT a delete: it never enqueues an outbox op, and the episode re-syncs on the next
    /// server pull / pull-to-refresh.
    RemoveLocalEpisodeData { episode_id: i32 },
    /// Remove ALL locally-cached data for a podcast and every one of its episodes —
    /// metadata, playbacks, device audio, playlist/queue membership — WITHOUT
    /// unsubscribing on the server (unlike [`Command::Unsubscribe`]). Local cache
    /// prune only: never enqueues an outbox op; everything re-syncs on the next pull.
    RemoveLocalPodcastData { podcast_id: i32 },
    /// INTERNAL — the connectivity WebSocket driver confirmed a live connection (a
    /// first pong came back). Flips connectivity Online and triggers an immediate
    /// resync pull, so reconnecting is reflected in ~1s instead of at the next 60s
    /// tick.
    WsConnected,
    /// INTERNAL — the connectivity WebSocket dropped, errored, went silent, or a
    /// (re)connect attempt failed. Flips connectivity Offline.
    WsDisconnected,
    /// INTERNAL — a pong round-trip completed; `rtt_ms` is one sample fed into the
    /// smoothed latency that drives the Online vs Degraded (yellow) tier.
    WsLatency { rtt_ms: u32 },
    /// INTERNAL — the WS ticket mint returned 401: the session is dead. Flags
    /// `auth_expired` so the provider signs out (same path as a 401 on pull).
    WsAuthExpired,
    /// Wipe all locally-cached data and reset in-memory state (sign-out).
    WipeLocal,
    /// Drop auth (the `ApiClient`) without wiping the local cache, e.g. after a
    /// 401. Stops the worker retrying a dead token; the local cache survives for
    /// the next sign-in.
    SignOut,
}

impl Command {
    /// Publish user-visible optimistic mutations before awaiting the outbox drain so slow networking cannot delay
    /// feedback. Skip commands that publish internally, carry no visible mutation, or report frequent task progress.
    /// New user mutations normally need early publication.
    pub fn reflects_optimistic_state(&self) -> bool {
        use Command::*;
        match self {
            // Self-publishing cache / ensure / load commands — they publish from
            // inside their handler, so an early publish would only double it.
            CacheEpisodes { .. }
            | CachePodcasts { .. }
            | CachePlaylists { .. }
            | CacheAutoPlaylists { .. }
            | EnsureDefaultPlaylist
            | LoadHistory { .. }
            | EnsurePodcast { .. }
            | EnsureEpisodeChapters { .. }
            | UpdatePlaylist { .. }
            | UpdatePodcastConfig { .. }
            | RemovePodcastConfig { .. } => false,

            // Internal task-reported / high-frequency progress commands.
            DeviceFetchProgress { .. }
            | DeviceFetchComplete { .. }
            | ServerDownloadProgress { .. }
            | ServerDownloadComplete { .. }
            | PodcastFetched { .. }
            | PodcastsFetched { .. }
            | EpisodeChaptersFetched { .. } => false,

            // Connectivity / auth / lifecycle commands that route through
            // do_pull / self-publish for their status, or carry no visible state.
            RefreshNow
            | SetAuth { .. }
            | SetOffline(_)
            | SetAddToQueueFront(_)
            | SetDownloadPrefs { .. }
            | SignOut
            | WipeLocal
            | WsConnected
            | WsDisconnected
            | WsLatency { .. }
            | WsAuthExpired => false,

            // Everything else is a user-initiated optimistic mutation (device /
            // server download, playlist edit, mark-played, local-data prune, …):
            // surface its state before the drain so feedback is immediate.
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, RedactedToken};

    #[test]
    fn optimistic_user_mutations_reflect_early() {
        // User-initiated commands whose optimistic local state must be visible
        // before the (possibly slow) drain.
        let early = [
            Command::DownloadToDevice {
                episode_ids: vec![1],
            },
            Command::RemoveDownload {
                episode_ids: vec![1],
            },
            Command::DownloadOnServer {
                episode_ids: vec![1],
            },
            Command::RemoveServerDownload {
                episode_ids: vec![1],
            },
            Command::DownloadToDevice {
                episode_ids: vec![1, 2],
            },
            Command::MarkPlayed {
                episode_id: 1,
                played: true,
            },
            Command::SetCursor {
                episode_id: 1,
                cursor: 5,
            },
            Command::AddToPlaylist {
                playlist_id: 1,
                episode_ids: vec![1],
            },
            Command::Subscribe {
                feed_url: "https://example.com/feed".into(),
                title: None,
                description: None,
                author: None,
            },
            Command::Unsubscribe { podcast_id: 1 },
            Command::RemoveLocalEpisodeData { episode_id: 1 },
            Command::RemoveLocalPodcastData { podcast_id: 1 },
        ];
        for cmd in early {
            assert!(
                cmd.reflects_optimistic_state(),
                "{cmd:?} should reflect optimistic state early"
            );
        }
    }

    #[test]
    fn self_publishing_and_internal_commands_do_not_reflect_early() {
        // Self-publishing (cache/load), high-frequency progress reports, and
        // connectivity/lifecycle commands must NOT trigger the extra publish.
        let late = [
            Command::CacheEpisodes { episodes: vec![] },
            Command::CachePodcasts { podcasts: vec![] },
            Command::EnsureDefaultPlaylist,
            Command::LoadHistory { reset: false },
            Command::EnsurePodcast { podcast_id: 1 },
            Command::DeviceFetchProgress {
                episode_id: 1,
                percent: 50,
            },
            Command::DeviceFetchComplete {
                episode_id: 1,
                result: Ok(()),
            },
            Command::ServerDownloadProgress {
                episode_id: 1,
                percent: 50,
            },
            Command::ServerDownloadComplete {
                episode_id: 1,
                episode: None,
            },
            Command::PodcastFetched {
                podcast_id: 1,
                podcast: None,
            },
            Command::EnsureEpisodeChapters { episode_id: 1 },
            Command::EpisodeChaptersFetched {
                episode_id: 1,
                episode: None,
            },
            Command::RefreshNow,
            Command::SetOffline(true),
            Command::SetAddToQueueFront(true),
            Command::SetDownloadPrefs {
                chunk_bytes: Some(4 * 1024 * 1024),
                parallelism: 1,
            },
            Command::SetAuth {
                server_url: "https://example.com".into(),
                token: RedactedToken("secret".into()),
            },
            Command::WsConnected,
            Command::WsDisconnected,
            Command::WsLatency { rtt_ms: 12 },
            Command::WsAuthExpired,
            Command::WipeLocal,
            Command::SignOut,
        ];
        for cmd in late {
            assert!(
                !cmd.reflects_optimistic_state(),
                "{cmd:?} should NOT reflect optimistic state early"
            );
        }
    }
}
