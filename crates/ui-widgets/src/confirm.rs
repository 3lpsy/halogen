//! Shared destructive-confirmation host.
//!
//! Confirmations for destructive actions render the same way: a single modal lives
//! at the app root ([`ConfirmHost`] in `AppLayout`), driven by a context controller
//! ([`Confirm`]) — exactly the pattern
//! [`QuickContextMenuHost`](crate::QuickContextMenuHost) uses. A
//! menu/row opens it with the target:
//!
//! ```ignore
//! let confirm = use_confirm();
//! // in a menu callback:
//! confirm.purge_episode(episode_id);            // local-only cache prune
//! confirm.delete_podcast(podcast_id);           // server-side unsubscribe
//! confirm.delete_podcast_then(id, on_deleted);  // …then navigate away
//! confirm.delete_playlist(playlist_id);         // server-side playlist delete
//! ```
//!
//! Hosting at the root (not per row) is deliberate: episode lists render on many
//! pages, so a per-row modal is the wrong shape, and root-hosting avoids the
//! per-page signal plumbing each call site would otherwise carry. Like
//! [`QuickMenu`](crate::QuickMenu)
//! it stores the caller's `on_deleted` callback and fires it on confirm.
//!
//! The *purge* variants dispatch a LOCAL-ONLY prune (no server call, no outbox; the
//! data re-syncs on the next pull) — recovery for a bad local cache. The *delete*
//! variant unsubscribes server-side. All render through the shared
//! [`ConfirmModal`](crate::ConfirmModal).

use std::rc::Rc;

use dioxus::prelude::*;

use crate::ConfirmModal;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::use_dispatch;

/// What a pending confirmation targets.
///
/// `on_deleted` is a scope-independent `Rc` closure, not a scope-owned
/// [`Callback`]: this target sits in a root-level signal while the modal is up,
/// and the scope that opened it (a list row, a detail page mid-navigation) can
/// unmount underneath — a `Callback` slot would die with it and panic on fire.
#[derive(Clone)]
pub enum ConfirmTarget {
    /// Remove one episode's local data (cache prune; server untouched).
    PurgeEpisode(i32),
    /// Remove a podcast's (and all its episodes') local data (server untouched).
    PurgePodcast(i32),
    /// Unsubscribe a podcast server-side (destructive). `on_deleted` fires after the
    /// dispatch — detail pages pass it to navigate away; list pages omit it.
    DeletePodcast {
        id: i32,
        on_deleted: Option<Rc<dyn Fn(i32)>>,
    },
    /// Delete a playlist server-side (destructive). `on_deleted` fires after the
    /// dispatch — detail pages pass it to navigate away; list pages omit it.
    DeletePlaylist {
        id: i32,
        on_deleted: Option<Rc<dyn Fn(i32)>>,
    },
}

/// Context handle for opening the shared confirm modal. `Copy`, so it can be grabbed
/// via [`use_confirm`] and used inside event handlers freely.
#[derive(Clone, Copy)]
pub struct Confirm(Signal<Option<ConfirmTarget>>);

impl Confirm {
    pub fn new() -> Self {
        Self(Signal::new(None))
    }

    /// Ask to remove one episode's local data.
    pub fn purge_episode(&self, episode_id: i32) {
        let mut target = self.0;
        target.set(Some(ConfirmTarget::PurgeEpisode(episode_id)));
    }

    /// Ask to remove a podcast's (and all its episodes') local data.
    pub fn purge_podcast(&self, podcast_id: i32) {
        let mut target = self.0;
        target.set(Some(ConfirmTarget::PurgePodcast(podcast_id)));
    }

    /// Ask to delete (unsubscribe) a podcast server-side.
    pub fn delete_podcast(&self, podcast_id: i32) {
        let mut target = self.0;
        target.set(Some(ConfirmTarget::DeletePodcast {
            id: podcast_id,
            on_deleted: None,
        }));
    }

    /// Delete a podcast, then run `on_deleted` (e.g. navigate away from its detail page).
    pub fn delete_podcast_then(&self, podcast_id: i32, on_deleted: Rc<dyn Fn(i32)>) {
        let mut target = self.0;
        target.set(Some(ConfirmTarget::DeletePodcast {
            id: podcast_id,
            on_deleted: Some(on_deleted),
        }));
    }

    /// Ask to delete a playlist server-side.
    pub fn delete_playlist(&self, playlist_id: i32) {
        let mut target = self.0;
        target.set(Some(ConfirmTarget::DeletePlaylist {
            id: playlist_id,
            on_deleted: None,
        }));
    }

    /// Delete a playlist, then run `on_deleted` (e.g. navigate away from its detail page).
    pub fn delete_playlist_then(&self, playlist_id: i32, on_deleted: Rc<dyn Fn(i32)>) {
        let mut target = self.0;
        target.set(Some(ConfirmTarget::DeletePlaylist {
            id: playlist_id,
            on_deleted: Some(on_deleted),
        }));
    }

    /// Dismiss without acting.
    pub fn close(&self) {
        let mut target = self.0;
        target.set(None);
    }

    /// Pre-bound closure form of [`purge_episode`](Self::purge_episode) — what
    /// menu/kebab builders want (their actions are scope-independent `Rc`
    /// closures — see [`QuickAction`](crate::QuickAction)), so call sites write
    /// `confirm.purge_episode_callback(id)` instead of hand-rolling the closure
    /// each time. `Confirm` is `Copy` and context-backed, so the closure can
    /// never dangle.
    pub fn purge_episode_callback(&self, episode_id: i32) -> Rc<dyn Fn()> {
        let me = *self;
        Rc::new(move || me.purge_episode(episode_id))
    }

    /// Pre-bound closure form of [`purge_podcast`](Self::purge_podcast).
    pub fn purge_podcast_callback(&self, podcast_id: i32) -> Rc<dyn Fn()> {
        let me = *self;
        Rc::new(move || me.purge_podcast(podcast_id))
    }

    /// Pre-bound closure form of [`delete_podcast`](Self::delete_podcast).
    pub fn delete_podcast_callback(&self, podcast_id: i32) -> Rc<dyn Fn()> {
        let me = *self;
        Rc::new(move || me.delete_podcast(podcast_id))
    }

    /// Pre-bound closure form of [`delete_podcast_then`](Self::delete_podcast_then).
    /// `on_deleted` must itself capture only scope-independent handles.
    pub fn delete_podcast_then_callback(
        &self,
        podcast_id: i32,
        on_deleted: impl Fn(i32) + 'static,
    ) -> Rc<dyn Fn()> {
        let me = *self;
        let on_deleted: Rc<dyn Fn(i32)> = Rc::new(on_deleted);
        Rc::new(move || me.delete_podcast_then(podcast_id, on_deleted.clone()))
    }

    /// Pre-bound closure form of [`delete_playlist`](Self::delete_playlist).
    pub fn delete_playlist_callback(&self, playlist_id: i32) -> Rc<dyn Fn()> {
        let me = *self;
        Rc::new(move || me.delete_playlist(playlist_id))
    }

    /// Pre-bound closure form of [`delete_playlist_then`](Self::delete_playlist_then).
    /// `on_deleted` must itself capture only scope-independent handles.
    pub fn delete_playlist_then_callback(
        &self,
        playlist_id: i32,
        on_deleted: impl Fn(i32) + 'static,
    ) -> Rc<dyn Fn()> {
        let me = *self;
        let on_deleted: Rc<dyn Fn(i32)> = Rc::new(on_deleted);
        Rc::new(move || me.delete_playlist_then(playlist_id, on_deleted.clone()))
    }
}

impl Default for Confirm {
    fn default() -> Self {
        Self::new()
    }
}

/// Read the shared [`Confirm`] controller from context.
pub fn use_confirm() -> Confirm {
    use_context::<Confirm>()
}

/// The single destructive-confirm modal host. Render once near the app root (the
/// [`Confirm`] context must already be provided by the parent). Renders only when a
/// target is set; the copy is tailored per variant.
#[component]
pub fn ConfirmHost() -> Element {
    let confirm = use_confirm();
    let Confirm(target) = confirm;
    let dispatch = use_dispatch();

    // Nothing pending → render nothing. (Hooks above run every render, so this early
    // return keeps a consistent hook order.)
    let Some(t) = target() else {
        return rsx! {};
    };

    // Copy + confirm label tailored per variant.
    let (title, body, confirm_label, title_id) = match t {
        ConfirmTarget::PurgeEpisode(_) => (
            "Remove local data?",
            "This removes this episode's downloaded audio, playback progress, and cached data from this device only. The server isn't touched — it re-syncs on the next refresh.",
            "Remove",
            "confirm-purge-title",
        ),
        ConfirmTarget::PurgePodcast(_) => (
            "Remove local data?",
            "This removes this podcast and all of its episodes' downloaded audio, playback progress, and cached data from this device only. The server isn't touched (you stay subscribed) — it re-syncs on the next refresh.",
            "Remove",
            "confirm-purge-title",
        ),
        ConfirmTarget::DeletePodcast { .. } => (
            "Delete podcast?",
            "This permanently removes the podcast and all of its episodes, downloads, and playback history. This can't be undone.",
            "Delete",
            "confirm-delete-podcast-title",
        ),
        ConfirmTarget::DeletePlaylist { .. } => (
            "Delete playlist?",
            "This permanently removes the playlist for every device. Its episodes stay in your library.",
            "Delete",
            "confirm-delete-playlist-title",
        ),
    };

    rsx! {
        ConfirmModal {
            title,
            body,
            confirm_label,
            danger: true,
            title_id,
            on_cancel: move |_| confirm.close(),
            on_confirm: move |_| {
                match &t {
                    ConfirmTarget::PurgeEpisode(id) => {
                        commands::remove_local_episode_data(&dispatch, *id)
                    }
                    ConfirmTarget::PurgePodcast(id) => {
                        commands::remove_local_podcast_data(&dispatch, *id)
                    }
                    ConfirmTarget::DeletePodcast { id, on_deleted } => {
                        commands::unsubscribe(&dispatch, *id);
                        if let Some(cb) = on_deleted {
                            cb(*id);
                        }
                    }
                    ConfirmTarget::DeletePlaylist { id, on_deleted } => {
                        commands::delete_playlist(&dispatch, *id);
                        if let Some(cb) = on_deleted {
                            cb(*id);
                        }
                    }
                }
                confirm.close();
            },
        }
    }
}
