use dioxus::prelude::*;

use crate::components::{ListSource, OrderDirection, SortField, SortSpec};
use halogen_webui_app_state::QueueState;
use halogen_webui_commands::actions as commands;
use halogen_webui_hooks::{use_config, use_dispatch, use_is_offline, use_playlists};

use super::ListPage;

/// Queue page — the default playlist, in `position` order. The queue *is* the
/// `is_default` playlist and may not exist, so this is a tri-state: resolving
/// (`Unknown`) shows a spinner, `Absent` prompts the user to create one, and
/// `Present` renders its episodes.
#[component]
pub fn Queue() -> Element {
    let playlists = use_playlists();
    let dispatch = use_dispatch();
    let config = use_config();

    let is_offline = use_is_offline();

    // Nudge authoritative queue resolution once per online stretch using a nonreactive guard. Re-arm offline so
    // reconnection retries Unknown, without dispatching on every state publication.
    let mut resolve_requested = use_signal(|| false);
    use_effect(move || {
        if is_offline() {
            resolve_requested.set(false);
            return;
        }
        if *resolve_requested.peek() {
            return;
        }
        resolve_requested.set(true);
        if matches!(playlists.peek().queue, QueueState::Unknown) {
            commands::ensure_default_playlist(&dispatch);
        }
    });

    match playlists.read().queue {
        QueueState::Present(id) => rsx! {
            ListPage {
                view_key: "queue",
                source: ListSource::Playlist { id },
                default_sort: SortSpec {
                    field: SortField::Custom,
                    direction: OrderDirection::Asc,
                },
                swipe: config().swipe_prefs.queue,
            }
        },
        QueueState::Absent => rsx! { QueueEmpty {} },
        // Unknown + KNOWN offline (cold-start SyncStatus::Unknown falls through to
        // the spinner): resolution can't complete — say so instead of spinning.
        QueueState::Unknown if is_offline() => rsx! {
            div { class: "p-8 max-w-md mx-auto text-center text-muted",
                h2 { class: "text-lg font-semibold text-base-content", "Queue unavailable offline" }
                p { class: "text-sm mt-1",
                    "You're offline and your queue hasn't been synced to this device yet — reconnect once to load it."
                }
            }
        },
        QueueState::Unknown => rsx! {
            div { class: "p-8 text-center text-muted",
                span { class: "loading loading-spinner loading-md" }
            }
        },
    }
}

/// Shown when the user has no queue (default playlist). Creating one from here
/// lands on the create form, which locks it to "make default".
#[component]
fn QueueEmpty() -> Element {
    let nav = use_navigator();
    rsx! {
        div { class: "p-8 max-w-md mx-auto text-center flex flex-col items-center gap-4",
            div {
                h2 { class: "text-lg font-semibold", "No queue yet" }
                p { class: "text-sm text-muted mt-1",
                    "Your queue is a playlist marked as the default. Create one to start queuing episodes."
                }
            }
            button {
                class: "btn btn-primary",
                onclick: move |_| {
                    nav.push("/playlists/create");
                },
                "Create your queue"
            }
        }
    }
}
