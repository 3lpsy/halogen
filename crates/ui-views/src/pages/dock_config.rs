//! Configure-dock screen (`/settings/dock`).
//!
//! Frontend-only editor for the shared nav order + visibility (`ClientConfig.nav`).
//! Drag the grips to reorder, toggle each item's visibility, then **Apply** to
//! persist to localStorage — the dock (first N visible), sidebar, and `/menu` all
//! re-render off the config signal.
//!
//! Rules: **Settings can never be hidden** (its toggle is locked on), and the dock
//! always keeps its **More** slot → `/menu`, so every visible item stays reachable.
//! Admin-only destinations (Polling, Server Logs) only appear for admins; pins and
//! any non-configurable keys are carried through untouched on save.

use std::collections::HashSet;

use dioxus::prelude::*;

use crate::components::{BackButton, nav_icon, nav_label, start_drag_reorder};
use halogen_ui_config::ClientConfigStore;
use halogen_ui_config::{BuiltinNav, NavConfig, NavKey};
use halogen_ui_icons::GripVertical;
use halogen_ui_state::hooks::{use_config, use_is_admin, use_toast};

const SETTINGS_KEY: NavKey = NavKey::Builtin(BuiltinNav::Settings);

#[cfg(test)]
mod tests {
    use super::*;

    // The admin-flip regression: seed the working copy with `admin = false`
    // (admin-only keys excluded), then apply — the merged order must still
    // contain every stored key, whatever `admin` says at apply time.
    #[test]
    fn merge_preserves_unmanaged_keys_across_admin_flip() {
        let stored = NavConfig::default();
        let working = configurable_order(&stored, false);
        assert!(
            working.len() < stored.order.len(),
            "seed excluded admin keys"
        );
        let merged = merge_nav_keys(&working, &stored.order);
        for key in &stored.order {
            assert!(merged.contains(key), "dropped {key:?}");
        }
        assert_eq!(merged.len(), stored.order.len());
    }
}

/// The stored keys this user can see and manage on this screen, in stored order —
/// the page's working copy.
fn configurable_order(nav: &NavConfig, admin: bool) -> Vec<NavKey> {
    nav.order
        .iter()
        .filter(|key| nav_label(key, admin).is_some())
        .cloned()
        .collect()
}

/// Merge the page's (possibly reordered) working copy back over the stored order:
/// the shown rows first, then everything the page didn't manage (pins, admin-only
/// items a non-admin can't touch) carried through in their stored relative order.
///
/// Carry-through is a plain set-difference against the working copy — NOT a
/// re-evaluation of `nav_label(key, admin)` with the live flag: if `is_admin`
/// flips after the working copy was seeded, the live-flag predicate would neither
/// show nor carry the admin-only keys, silently dropping them from the persisted
/// order. The result always contains every key of `stored`.
fn merge_nav_keys(working: &[NavKey], stored: &[NavKey]) -> Vec<NavKey> {
    let mut merged = working.to_vec();
    for key in stored {
        if !merged.contains(key) {
            merged.push(key.clone());
        }
    }
    merged
}

#[component]
pub fn ConfigureDock() -> Element {
    let mut config = use_config();
    let is_admin = use_is_admin();
    let toast = use_toast();
    let admin = is_admin();

    // Working copies (committed only on Apply). Seeded once from config:
    //  - `order`: the builtins this user can configure here, in their stored order.
    //  - `hidden`: which of them are hidden (Settings forced visible).
    let mut order = use_signal(|| configurable_order(&config.peek().nav, admin));
    // `is_admin` resolves asynchronously after login; if it flips while this page
    // is mounted, re-seed the working copy so the admin-only rows appear (and get
    // carried in `order` rather than depending on the apply-time carry-through).
    // Un-applied edits from the brief pre-flip window are reset — the flip
    // happens once, right after login.
    use_effect(use_reactive!(|admin| {
        order.set(configurable_order(&config.peek().nav, admin));
    }));
    let mut hidden = use_signal(|| {
        let mut h = HashSet::new();
        for key in &config.peek().nav.hidden {
            if *key != SETTINGS_KEY {
                h.insert(key.clone());
            }
        }
        h
    });

    let on_apply = move |_| {
        let stored = config.peek().nav.clone();
        // Reordered configurable builtins, then everything we didn't show carried
        // through — see `merge_nav_keys` for why this must not consult `admin`.
        let new_order = merge_nav_keys(&order.read(), &stored.order);
        // The working `hidden` set is seeded from ALL of `config.nav.hidden`
        // (`:96`), so it already IS the complete desired hidden set — persist it
        // directly. Merging with `stored.hidden` the way `order` does would
        // re-add every key the user just un-hid (the only un-hide path is
        // removing from the working set), making un-hiding impossible.
        let mut new_hidden: Vec<NavKey> = hidden.read().iter().cloned().collect();
        // Settings is never hidden.
        new_hidden.retain(|k| *k != SETTINGS_KEY);

        {
            let mut w = config.write();
            w.nav.order = new_order;
            w.nav.hidden = new_hidden;
        }
        let to_save = config.peek().clone();
        spawn(async move {
            ClientConfigStore::save(&to_save).await;
        });
        toast.success("Dock updated.");
    };

    let rows = order.read().clone();

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row.
            div { class: "p-2", BackButton {} }
            // Scrollable list.
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 space-y-3",
                h1 { class: "text-2xl font-bold", "Configure Dock" }
                p { class: "text-base-content/60 text-sm",
                    "Drag to reorder; the dock shows the first few in order. Toggle to show or hide an item across the dock, sidebar, and menu. The More button is always present."
                }
                div { id: "dock-config-list", class: "flex flex-col gap-2",
                    for (i, key) in rows.iter().enumerate() {
                        {
                            let key = key.clone();
                            let label = nav_label(&key, admin).unwrap_or_default();
                            let is_settings = key == SETTINGS_KEY;
                            let is_visible = !hidden.read().contains(&key);
                            let toggle_key = key.clone();
                            rsx! {
                                div {
                                    key: "{key:?}",
                                    id: "dock-row-{i}",
                                    "data-nav-index": "{i}",
                                    class: "flex items-center gap-2 p-2 bg-base-200 rounded-lg select-none",
                                    // Drag grip — `touch-none` so dragging it doesn't scroll
                                    // the page; the whole gesture runs in JS (see
                                    // `start_drag_reorder`, shared with the playlist + episode lists).
                                    div {
                                        class: "cursor-grab touch-none text-base-content/40 hover:text-base-content/70 px-1",
                                        "aria-label": "Drag to reorder",
                                        onpointerdown: move |e: PointerEvent| {
                                            e.stop_propagation();
                                            let from = i;
                                            start_drag_reorder(
                                                "dock-config-list",
                                                "data-nav-index",
                                                &format!("dock-row-{from}"),
                                                from,
                                                e.client_coordinates().y,
                                                move |to| {
                                                    let mut o = order.write();
                                                    if from < o.len() {
                                                        let item = o.remove(from);
                                                        let pos = to.min(o.len());
                                                        o.insert(pos, item);
                                                    }
                                                },
                                            );
                                        },
                                        GripVertical { class: "w-5 h-5" }
                                    }
                                    {nav_icon(&key, "w-5 h-5")}
                                    span { class: "flex-1 font-medium", "{label}" }
                                    // Visibility toggle — checked = shown. Settings is locked on.
                                    input {
                                        r#type: "checkbox",
                                        class: "toggle toggle-primary toggle-sm",
                                        "aria-label": "Show {label}",
                                        checked: is_visible,
                                        disabled: is_settings,
                                        onchange: move |e| {
                                            let mut h = hidden.write();
                                            if e.checked() {
                                                h.remove(&toggle_key);
                                            } else {
                                                h.insert(toggle_key.clone());
                                            }
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Pinned bottom action bar — Apply pulled to the right.
            div { class: "p-2 border-t border-base-300 flex justify-end",
                button {
                    class: "btn btn-primary",
                    r#type: "button",
                    onclick: on_apply,
                    "Apply"
                }
            }
        }
    }
}
