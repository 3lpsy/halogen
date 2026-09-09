//! Smart-reorder screen (`/playlists/:id/reorder-by`). Pick a sort field + direction; on Apply the playlist's `Custom`
//! (position) order is rewritten to match (offline-capable optimistic reorder + outbox, see
//! [`commands::reorder_playlist`](halogen_webui_commands::actions::reorder_playlist)). The user can still drag to
//! fine-tune afterwards.

use dioxus::prelude::*;
use halogen_wire::{OrderDirection, PlaylistReorderField};

use crate::components::BackButton;
use halogen_webui_commands::actions as commands;
use halogen_webui_hooks::{
    use_config, use_deep_link_resource, use_dispatch, use_playlists, use_toast,
};

#[component]
pub fn PlaylistReorderBy(id: i32) -> Element {
    let playlists = use_playlists();
    let config = use_config();
    let dispatch = use_dispatch();
    let toast = use_toast();
    let nav = use_navigator();

    let mut field = use_signal(|| PlaylistReorderField::Published);
    let mut direction = use_signal(|| OrderDirection::Desc);

    // Validate the playlist exists before letting the user reorder it — a deep link
    // to `/playlists/:id/reorder-by` for an unknown id would otherwise dispatch a
    // reorder against a playlist that was never confirmed to exist. Mirrors
    // `PlaylistDetail`.
    let load_failed = use_deep_link_resource(
        config,
        move || playlists.read().playlists.iter().any(|p| p.id == id),
        move |client| async move {
            let pl = client.get_playlist(id).await.map_err(|e| e.to_string())?;
            commands::cache_playlists(&dispatch, vec![pl]);
            Ok(())
        },
    );

    let name = playlists
        .read()
        .playlists
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.clone());
    let exists = name.is_some();

    let on_apply = move |_| {
        // Never reorder an unvalidated/unknown playlist.
        if !exists {
            return;
        }
        commands::reorder_playlist(&dispatch, id, field(), direction());
        toast.success("Playlist reordered.");
        nav.replace(format!("/playlists/{id}", id = id));
    };

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row.
            div { class: "p-2", BackButton {} }
            // Scrollable form.
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 space-y-4",
                h1 { class: "text-2xl font-bold", "Reorder" }
                p { class: "text-base-content/60 text-sm",
                    if let Some(n) = name.as_ref() {
                        "Smart-reorder \"{n}\" by a field. This rewrites the custom order; you can still drag to fine-tune afterwards."
                    } else if load_failed() {
                        "Playlist not found."
                    } else {
                        "Loading…"
                    }
                }

                div { class: "space-y-2",
                    label { class: "text-muted", "Sort by" }
                    select {
                        "aria-label": "Sort field",
                        class: "select select-bordered w-full",
                        value: "{field().as_str()}",
                        onchange: move |e| field.set(PlaylistReorderField::from_str_or_default(&e.value())),
                        for f in PlaylistReorderField::ALL {
                            option { value: f.as_str(), selected: f == field(), "{f.label()}" }
                        }
                    }
                }
                div { class: "space-y-2",
                    label { class: "text-muted", "Direction" }
                    select {
                        "aria-label": "Sort direction",
                        class: "select select-bordered w-full",
                        value: if direction() == OrderDirection::Desc { "desc" } else { "asc" },
                        onchange: move |e| {
                            direction.set(if e.value() == "desc" {
                                OrderDirection::Desc
                            } else {
                                OrderDirection::Asc
                            });
                        },
                        option { value: "asc", selected: direction() == OrderDirection::Asc, "Ascending" }
                        option { value: "desc", selected: direction() == OrderDirection::Desc, "Descending" }
                    }
                }
            }
            // Pinned bottom action bar.
            div { class: "p-2 border-t border-base-300 flex justify-end",
                button {
                    class: "btn btn-primary",
                    r#type: "button",
                    disabled: !exists,
                    onclick: on_apply,
                    "Apply"
                }
            }
        }
    }
}
