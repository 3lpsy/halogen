//! Queue/playlist/podcast navigation helpers for the
//! [`PlayerController`](super::PlayerController).
//!
//! Free functions over `&EpisodeState` + the current episode id, kept out of the
//! controller so the playback state machine isn't also the place that knows how
//! to walk the active list. The controller's transport methods
//! (`play_next_episode`/`play_previous_episode`/`has_next`/`has_adjacent`) are
//! thin wrappers that resolve a target here and then route it through
//! `request_play`.

use halogen_ui_appstate::{EpisodeState, PlaylistState};

/// The episode `delta` (+1/-1) places away from `current` in the active list:
/// the play-`context` playlist when `current` is in it, else the default queue
/// playlist when `current` is in it, otherwise the episode's podcast list
/// (server order). `None` at the ends of the list.
///
/// Reads the episode pool from `EpisodeState` and the queue/membership from the
/// `PlaylistState` slice.
pub fn adjacent_episode(
    episodes: &EpisodeState,
    playlists: &PlaylistState,
    current: i32,
    delta: i32,
    context: Option<i32>,
) -> Option<i32> {
    let neighbor = |ids: &[i32]| -> Option<i32> {
        let idx = ids.iter().position(|&id| id == current)?;
        let target = idx as i32 + delta;
        if target < 0 {
            return None;
        }
        ids.get(target as usize).copied()
    };
    // The play context (the playlist the user pressed play from) wins when the
    // current episode is still in it.
    if let Some(ids) = context.and_then(|pid| playlists.episodes_by_playlist.get(&pid))
        && ids.contains(&current)
    {
        return neighbor(ids);
    }
    // Resolve the queue via `queue_id()` (authoritative), not by scanning the
    // now-partial playlists pool for the default.
    let queue = playlists
        .queue_id()
        .and_then(|qid| playlists.episodes_by_playlist.get(&qid));
    if let Some(ids) = queue
        && ids.contains(&current)
    {
        return neighbor(ids);
    }
    let podcast_id = episodes.episodes_by_id.get(&current)?.podcast_id;
    episodes
        .episodes_by_podcast
        .get(&podcast_id)
        .and_then(|ids| neighbor(ids))
}

/// The transport Next target: the playlist/queue continuation
/// ([`PlaylistState::next_up_in`] with the play `context`) when there is one,
/// else the podcast-order neighbor so you can keep bingeing a podcast whose
/// episode isn't queued. Auto-advance (on episode end) uses `next_up_in`
/// directly — no podcast fallback — so it stops at the playlist's end.
pub fn next_episode(
    episodes: &EpisodeState,
    playlists: &PlaylistState,
    current: Option<i32>,
    context: Option<i32>,
) -> Option<i32> {
    let queued = playlists.next_up_in(current, context);
    queued.or_else(|| current.and_then(|c| adjacent_episode(episodes, playlists, c, 1, context)))
}
