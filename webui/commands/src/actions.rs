//! Typed UI write helpers send Commands to the sync worker for optimistic state and durable queuing. Components call
//! these helpers instead of constructing commands; reads use domain-state hooks.

use dioxus::prelude::Coroutine;
use halogen_wire::{
    EpisodeData, OrderDirection, PlaylistData, PlaylistReorderField, PlaylistUpdateData,
    PodcastConfigUpdateData, PodcastData,
};

use crate::{Command, RedactedToken};

/// Subscribe to a podcast feed by URL only (no metadata known yet).
pub fn subscribe(d: &Coroutine<Command>, feed_url: String) {
    d.send(Command::Subscribe {
        feed_url,
        title: None,
        description: None,
        author: None,
    });
}

/// Subscribe to a discovered podcast, carrying the directory metadata so the
/// created row has its real title/description/author immediately.
pub fn subscribe_discovered(
    d: &Coroutine<Command>,
    feed_url: String,
    title: String,
    description: Option<String>,
    author: Option<String>,
) {
    d.send(Command::Subscribe {
        feed_url,
        title: Some(title),
        description,
        author,
    });
}

/// Hand a freshly server-paged batch of episodes to the worker to persist and
/// merge into the in-memory indices (server-paged views like `/latest`).
pub fn cache_episodes(d: &Coroutine<Command>, episodes: Vec<EpisodeData>) {
    if episodes.is_empty() {
        return;
    }
    d.send(Command::CacheEpisodes { episodes });
}

/// Hand a freshly server-paged batch of podcasts to the worker to persist and
/// merge into `podcasts_by_id` (the paged podcasts list).
pub fn cache_podcasts(d: &Coroutine<Command>, podcasts: Vec<PodcastData>) {
    if podcasts.is_empty() {
        return;
    }
    d.send(Command::CachePodcasts { podcasts });
}

/// Upsert playlists into local state after a direct (non-outbox) create/update,
/// so navigation to the new playlist is instant. Pair with [`refresh`] to reconcile.
pub fn cache_playlists(d: &Coroutine<Command>, playlists: Vec<PlaylistData>) {
    if playlists.is_empty() {
        return;
    }
    d.send(Command::CachePlaylists { playlists });
}

/// Unsubscribe from a podcast.
pub fn unsubscribe(d: &Coroutine<Command>, podcast_id: i32) {
    d.send(Command::Unsubscribe { podcast_id });
}

/// Delete a playlist server-side (online-only — the worker calls the API
/// directly and toasts on failure; no outbox op).
pub fn delete_playlist(d: &Coroutine<Command>, playlist_id: i32) {
    d.send(Command::DeletePlaylist { playlist_id });
}

/// Ensure a podcast is loaded into the pool (lazy fetch on miss; the worker
/// de-dups, so it's safe to call from every episode row that needs the name).
pub fn ensure_podcast(d: &Coroutine<Command>, podcast_id: i32) {
    d.send(Command::EnsurePodcast { podcast_id });
}

/// Ensure an episode's chapters are loaded into the pool (lazy fetch on miss; the
/// worker de-dups, so it's safe to call whenever the expanded player opens or the
/// track changes).
pub fn ensure_episode_chapters(d: &Coroutine<Command>, episode_id: i32) {
    d.send(Command::EnsureEpisodeChapters { episode_id });
}

/// Download the given episodes to this device. Always bulk — pass `vec![id]` for a
/// single episode (the worker + outbox treat one and many identically).
pub fn download_to_device(d: &Coroutine<Command>, episode_ids: Vec<i32>) {
    d.send(Command::DownloadToDevice { episode_ids });
}

/// Remove the given episodes' device downloads.
pub fn remove_download(d: &Coroutine<Command>, episode_ids: Vec<i32>) {
    d.send(Command::RemoveDownload { episode_ids });
}

/// Force-fresh device redownload (remove + re-pull) for the given episodes.
pub fn redownload_device(d: &Coroutine<Command>, episode_ids: Vec<i32>) {
    d.send(Command::RedownloadDevice { episode_ids });
}

/// Trigger a server-side download only (no device download) for the given episodes.
pub fn download_on_server(d: &Coroutine<Command>, episode_ids: Vec<i32>) {
    d.send(Command::DownloadOnServer { episode_ids });
}

/// Remove the server's downloaded copies of the given episodes.
pub fn remove_server_download(d: &Coroutine<Command>, episode_ids: Vec<i32>) {
    d.send(Command::RemoveServerDownload { episode_ids });
}

/// Force-fresh server redownload (remove + download) for the given episodes.
pub fn redownload_on_server(d: &Coroutine<Command>, episode_ids: Vec<i32>) {
    d.send(Command::RedownloadOnServer { episode_ids });
}

/// Mark an episode played / unplayed.
pub fn mark_played(d: &Coroutine<Command>, episode_id: i32, played: bool) {
    d.send(Command::MarkPlayed { episode_id, played });
}

/// Set an episode's playback cursor (seconds). Used by the `ResetProgress` swipe
/// (cursor 0) — normal cursor persistence is sent directly by the player.
pub fn set_cursor(d: &Coroutine<Command>, episode_id: i32, cursor: i64) {
    d.send(Command::SetCursor { episode_id, cursor });
}

/// Add the given episodes to a playlist (e.g. the queue). Always bulk — pass
/// `vec![id]` for a single episode.
pub fn add_to_playlist(d: &Coroutine<Command>, playlist_id: i32, episode_ids: Vec<i32>) {
    d.send(Command::AddToPlaylist {
        playlist_id,
        episode_ids,
    });
}

/// Remove the given episodes from a playlist. Always bulk — pass `vec![id]` for a
/// single episode.
pub fn remove_from_playlist(d: &Coroutine<Command>, playlist_id: i32, episode_ids: Vec<i32>) {
    d.send(Command::RemoveFromPlaylist {
        playlist_id,
        episode_ids,
    });
}

/// Reorder an episode within a playlist to target index `to` (manual order).
/// `to` is a *stored* ascending position; callers working from a displayed list
/// that may be shown descending should use [`move_in_playlist_visual`].
pub fn move_in_playlist(d: &Coroutine<Command>, playlist_id: i32, episode_id: i32, to: i32) {
    d.send(Command::MoveInPlaylist {
        playlist_id,
        episode_id,
        to,
    });
}

/// Translate displayed reorder slots into stored ascending positions. Descending index v maps to full_len - 1 - v;
/// full_len is the unwindowed count, while ascending is unchanged.
pub fn move_in_playlist_visual(
    d: &Coroutine<Command>,
    playlist_id: i32,
    episode_id: i32,
    visual_to: i32,
    reversed: bool,
    full_len: usize,
) {
    move_in_playlist(
        d,
        playlist_id,
        episode_id,
        stored_index(visual_to, reversed, full_len),
    );
}

/// Translate a target slot from displayed-list coordinates to the stored ascending
/// position the server expects. Descending (down arrow) mirrors visual index `v` to
/// `full_len - 1 - v`; ascending is identity. Clamped non-negative so an
/// out-of-range index can never produce a bogus target. Pure — unit-tested below.
fn stored_index(visual_to: i32, reversed: bool, full_len: usize) -> i32 {
    if reversed {
        (full_len as i32 - 1 - visual_to).max(0)
    } else {
        visual_to
    }
}

/// Reorder a playlist within the user's manual order to target index `to`
/// (optimistic + outbox).
pub fn move_playlist(d: &Coroutine<Command>, playlist_id: i32, to: i32) {
    d.send(Command::MovePlaylist { playlist_id, to });
}

/// Smart-reorder a playlist's episodes by `field`/`direction` (optimistic + outbox).
pub fn reorder_playlist(
    d: &Coroutine<Command>,
    playlist_id: i32,
    field: PlaylistReorderField,
    direction: OrderDirection,
) {
    d.send(Command::ReorderPlaylist {
        playlist_id,
        field,
        direction,
    });
}

/// Resolve the queue (default playlist) via the targeted endpoint. Lazy/cheap;
/// dispatch when `EpisodeState.queue` is `Unknown` and you need it resolved.
pub fn ensure_default_playlist(d: &Coroutine<Command>) {
    d.send(Command::EnsureDefaultPlaylist);
}

/// Edit a playlist offline (optimistic + outbox). Online edits go direct instead.
pub fn update_playlist(d: &Coroutine<Command>, playlist_id: i32, data: PlaylistUpdateData) {
    d.send(Command::UpdatePlaylist { playlist_id, data });
}

/// Edit a podcast config offline (optimistic + outbox). Online edits go direct.
pub fn update_podcast_config(
    d: &Coroutine<Command>,
    podcast_id: i32,
    config_id: i32,
    data: PodcastConfigUpdateData,
) {
    d.send(Command::UpdatePodcastConfig {
        podcast_id,
        config_id,
        data,
    });
}

/// Remove a podcast config (optimistic unlink + outbox).
pub fn remove_podcast_config(d: &Coroutine<Command>, podcast_id: i32, config_id: i32) {
    d.send(Command::RemovePodcastConfig {
        podcast_id,
        config_id,
    });
}

/// Set a podcast's auto-add playlists offline (optimistic cache + outbox). Online
/// edits go direct, then call [`cache_auto_playlists`] instead. `add_to_start`
/// is the podcast's insert-position override (`None` = server default).
pub fn set_podcast_auto_playlists(
    d: &Coroutine<Command>,
    podcast_id: i32,
    playlist_ids: Vec<i32>,
    add_to_start: Option<bool>,
) {
    d.send(Command::SetPodcastAutoPlaylists {
        podcast_id,
        playlist_ids,
        add_to_start,
    });
}

/// Cache a podcast's auto-add playlist set after a direct (online) fetch or save.
pub fn cache_auto_playlists(
    d: &Coroutine<Command>,
    podcast_id: i32,
    playlist_ids: Vec<i32>,
    add_to_start: Option<bool>,
) {
    d.send(Command::CacheAutoPlaylists {
        podcast_id,
        playlist_ids,
        add_to_start,
    });
}

/// Force an immediate server refresh.
pub fn refresh(d: &Coroutine<Command>) {
    d.send(Command::RefreshNow);
}

/// Page the History source: `reset` re-fetches `GET /playbacks` from page 0
/// (mount / pull-to-refresh); otherwise advance to the next page (scroll).
pub fn load_history(d: &Coroutine<Command>, reset: bool) {
    d.send(Command::LoadHistory { reset });
}

/// Set manual "Go Offline" mode (the navbar status toggle + boot restore of the
/// persisted choice). `true` forces the worker offline; `false` reconnects.
pub fn set_offline(d: &Coroutine<Command>, offline: bool) {
    d.send(Command::SetOffline(offline));
}

/// Mirror the "add to front of queue" preference into the worker (boot restore +
/// whenever the setting changes). `true` inserts queue adds at position 0.
pub fn set_add_to_queue_front(d: &Coroutine<Command>, front: bool) {
    d.send(Command::SetAddToQueueFront(front));
}

/// Mirror the device-download preferences into the worker (boot restore + whenever
/// the setting changes). `chunk_bytes` is the per-request chunk size (`None` = no
/// chunking) and `parallelism` is the concurrent-chunk count within one download.
pub fn set_download_prefs(d: &Coroutine<Command>, chunk_bytes: Option<u64>, parallelism: u8) {
    d.send(Command::SetDownloadPrefs {
        chunk_bytes,
        parallelism,
    });
}

/// Remove all locally-cached data for one episode (metadata, playback, device
/// audio, playlist membership) WITHOUT touching the server. Local recovery only —
/// the episode re-syncs on the next pull. See [`Command::RemoveLocalEpisodeData`].
pub fn remove_local_episode_data(d: &Coroutine<Command>, episode_id: i32) {
    d.send(Command::RemoveLocalEpisodeData { episode_id });
}

/// Remove all locally-cached data for a podcast and its episodes WITHOUT
/// unsubscribing on the server. Local recovery only — it re-syncs on the next
/// pull. See [`Command::RemoveLocalPodcastData`].
pub fn remove_local_podcast_data(d: &Coroutine<Command>, podcast_id: i32) {
    d.send(Command::RemoveLocalPodcastData { podcast_id });
}

/// Wipe all locally-cached data (sign-out).
pub fn wipe_local(d: &Coroutine<Command>) {
    d.send(Command::WipeLocal);
}

/// Drop the worker's auth without wiping the local cache (e.g. after a 401).
pub fn sign_out(d: &Coroutine<Command>) {
    d.send(Command::SignOut);
}

/// Set/replace the worker's auth (after login or token refresh).
pub fn set_auth(d: &Coroutine<Command>, server_url: String, token: String) {
    d.send(Command::SetAuth {
        server_url,
        token: RedactedToken(token),
    });
}

#[cfg(test)]
mod tests {
    use super::stored_index;

    #[test]
    fn ascending_is_identity() {
        assert_eq!(stored_index(0, false, 5), 0);
        assert_eq!(stored_index(3, false, 5), 3);
        assert_eq!(stored_index(4, false, 5), 4);
    }

    #[test]
    fn descending_mirrors_against_full_len() {
        // len 5, descending: visual 0 is the highest stored position (4), and the
        // visual bottom (4) is stored 0; the middle is its own mirror.
        assert_eq!(stored_index(0, true, 5), 4);
        assert_eq!(stored_index(1, true, 5), 3);
        assert_eq!(stored_index(2, true, 5), 2);
        assert_eq!(stored_index(4, true, 5), 0);
    }

    #[test]
    fn descending_clamps_nonnegative() {
        // A target past the end must never mirror to a negative stored index.
        assert_eq!(stored_index(9, true, 5), 0);
        assert_eq!(stored_index(0, true, 0), 0);
    }
}
