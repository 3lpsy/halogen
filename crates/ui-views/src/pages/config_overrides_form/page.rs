//! Admin-only config-overrides editor (`/settings/config/overrides`).
//! See the crate-level module docs in `mod.rs`.

use dioxus::prelude::*;
use halogen_wire::ConfigOverridesData;

use super::params::{InputKind, PARAMS, ParamMeta, apply_value, current_value, meta_for};
use crate::Route;
use crate::components::{BackButton, ConfirmModal, FieldError};
use halogen_ui_icons::{MagnifyingGlass, XMark};
use halogen_ui_state::hooks::{
    use_config, use_confirm_action, use_is_admin, use_is_offline, use_toast,
};

/// One editable override row in the working set.
#[derive(Clone, PartialEq)]
struct Row {
    key: &'static str,
    value: String,
    error: Option<String>,
}

/// One active override: label/description, the type-appropriate input, and a
/// remove button. Mutates the shared working set by stable `key` (each param
/// appears at most once), so a concurrent remove can't shift the wrong row.
#[component]
fn OverrideRow(mut rows: Signal<Vec<Row>>, row: Row) -> Element {
    let meta = meta_for(row.key);
    let label = meta.map(|m| m.label).unwrap_or(row.key);
    let desc = meta.map(|m| m.desc).unwrap_or("");
    let kind = meta.map(|m| m.kind).unwrap_or(InputKind::Text);
    let key = row.key;
    let value = row.value.clone();
    let err = row.error.clone();

    // Set this row's value (clearing its error) by key, not by position.
    let mut set_value = move |new: String| {
        if let Some(r) = rows.write().iter_mut().find(|r| r.key == key) {
            r.value = new;
            r.error = None;
        }
    };

    rsx! {
        div { class: "p-3 bg-base-200 rounded-lg space-y-1",
            div { class: "flex items-start justify-between gap-2",
                div {
                    div { class: "font-medium text-sm", "{label}" }
                    div { class: "text-xs text-base-content/50", "{desc}" }
                }
                button {
                    "aria-label": "Remove override",
                    class: "btn btn-square btn-ghost btn-sm",
                    r#type: "button",
                    onclick: move |_| {
                        rows.write().retain(|r| r.key != key);
                    },
                    XMark { class: "w-4 h-4" }
                }
            }
            OverrideInput {
                label: label.to_string(),
                kind,
                value,
                on_value: move |new| set_value(new),
            }
            FieldError { messages: err.into_iter().collect() }
        }
    }
}

/// The type-appropriate control for one override row: a select (Bool), a date /
/// text / number `<input>` per [`InputKind`]. Unlike the shared [`InputField`]
/// (two-way `Signal<String>` + a label wrapper), this is string-keyed — it pushes
/// each change up through `on_value` so the parent can update the working set by
/// stable `key` — and renders bare (the row already owns the label + error). The
/// rendered markup/classes match the hand-written arms exactly.
#[component]
fn OverrideInput(
    label: String,
    kind: InputKind,
    value: String,
    on_value: EventHandler<String>,
) -> Element {
    rsx! {
        match kind {
            InputKind::Bool => rsx! {
                select {
                    "aria-label": "{label}",
                    class: "select select-bordered w-full",
                    value: "{value}",
                    onchange: move |e| on_value.call(e.value()),
                    option { value: "true", "Enabled" }
                    option { value: "false", "Disabled" }
                }
            },
            InputKind::Date => rsx! {
                input {
                    "aria-label": "{label}",
                    class: "input input-bordered w-full",
                    r#type: "date",
                    value: "{value}",
                    oninput: move |e| on_value.call(e.value()),
                }
            },
            InputKind::Text => rsx! {
                input {
                    "aria-label": "{label}",
                    class: "input input-bordered w-full",
                    r#type: "text",
                    value: "{value}",
                    oninput: move |e| on_value.call(e.value()),
                }
            },
            InputKind::Number | InputKind::Percent => rsx! {
                input {
                    "aria-label": "{label}",
                    class: "input input-bordered w-full",
                    r#type: "number",
                    min: "0",
                    max: if kind == InputKind::Percent { "100" } else { "" },
                    value: "{value}",
                    oninput: move |e| on_value.call(e.value()),
                }
            },
        }
    }
}

#[component]
pub fn ConfigOverridesEdit() -> Element {
    let cfg = use_config();
    let nav = use_navigator();
    let is_admin = use_is_admin();
    let toast = use_toast();

    // Working set + load/save state.
    let mut rows = use_signal(Vec::<Row>::new);
    let mut query = use_signal(String::new);
    let mut loaded_once = use_signal(|| false);
    let mut loading = use_signal(|| true);
    let mut load_error = use_signal(|| Option::<String>::None);
    let mut overrides_disabled = use_signal(|| false);
    let mut server_error = use_signal(|| Option::<String>::None);

    // Save confirm + clear-all confirm flows.
    let mut pending = use_signal(|| Option::<ConfigOverridesData>::None);
    let save = use_confirm_action();
    let clear = use_confirm_action();

    // Load the current overrides once (and learn whether the mechanism is
    // disabled, for a proactive notice). Guarded so it runs a single time.
    use_effect(move || {
        if loaded_once() {
            return;
        }
        loaded_once.set(true);
        let snapshot = cfg.peek().clone();
        spawn(async move {
            let client = match snapshot.api_client_or_err() {
                Ok(c) => c,
                Err(e) => {
                    load_error.set(Some(e));
                    loading.set(false);
                    return;
                }
            };
            match client.get_config_overrides().await {
                Ok(data) => {
                    let initial: Vec<Row> = PARAMS
                        .iter()
                        .filter_map(|p| {
                            current_value(&data, p.key).map(|value| Row {
                                key: p.key,
                                value,
                                error: None,
                            })
                        })
                        .collect();
                    rows.set(initial);
                }
                Err(e) => load_error.set(Some(e.to_string())),
            }
            // Best-effort: proactively show the disabled notice if applicable.
            if let Ok(c) = client.get_config().await {
                overrides_disabled.set(c.config_overrides_disabled);
            }
            loading.set(false);
        });
    });

    // `use_is_offline` is a hook, so it MUST run unconditionally — keep it ABOVE the
    // admin guard. `use_is_admin` resolves asynchronously and can flip false→true
    // while mounted; a hook after the early-returning guard would change the hook
    // count on that flip and corrupt Dioxus's hook indices.
    let is_offline = use_is_offline()();

    // All hooks are above this guard so they always run.
    if !is_admin() {
        return rsx! {
            div { class: "p-2",
                BackButton {}
                p { class: "text-base-content/60 mt-4", "Admin only." }
            }
        };
    }

    let mutate_disabled = is_offline || overrides_disabled();

    // Snapshot the working set for this render (avoids holding a read guard while
    // the row handlers write).
    let current_rows = rows.read().clone();
    let active_keys: Vec<&'static str> = current_rows.iter().map(|r| r.key).collect();

    // Inactive params matching the picker query.
    let q = query().to_lowercase();
    let suggestions: Vec<&'static ParamMeta> = PARAMS
        .iter()
        .filter(|p| !active_keys.contains(&p.key))
        .filter(|p| {
            q.is_empty()
                || p.label.to_lowercase().contains(q.as_str())
                || p.desc.to_lowercase().contains(q.as_str())
                || p.key.contains(q.as_str())
        })
        .collect();

    rsx! {
        div { class: "flex flex-col h-full overflow-hidden",
            // Pinned back-button row — stays put while the body scrolls.
            div { class: "p-2", BackButton {} }
            // Scrollable body beneath the pinned back button.
            div { class: "flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2 space-y-4",
            h1 { class: "text-2xl font-bold", "Config Overrides" }
            p { class: "text-base-content/60 text-sm",
                "Override the server's runtime config. Changes are saved to the overrides file and take effect after a server restart."
            }

            if is_offline {
                div { class: "alert alert-warning text-sm",
                    "You're offline — reconnect to edit overrides."
                }
            }
            if overrides_disabled() {
                div { class: "alert alert-warning text-sm",
                    "Config overrides are disabled on this server; changes can't be saved."
                }
            }

            if loading() {
                p { class: "text-base-content/60", "Loading…" }
            }
            if !loading() {
                if let Some(err) = load_error() {
                    div { class: "alert alert-error", "Could not load overrides: {err}" }
                } else {
                // ── Add a parameter (search + clickable options) — at the top ──
                div { class: "space-y-2",
                    label { class: "text-sm font-medium", "Add an override" }
                    div { class: "relative",
                        span { class: "absolute left-3 top-1/2 -translate-y-1/2 text-base-content/40",
                            MagnifyingGlass { class: "w-4 h-4" }
                        }
                        input {
                            "aria-label": "Search parameters",
                            class: "input input-bordered w-full pl-10",
                            r#type: "text",
                            placeholder: "Search parameters…",
                            value: "{query}",
                            oninput: move |e| query.set(e.value()),
                        }
                    }
                    if suggestions.is_empty() {
                        p { class: "text-xs text-base-content/40", "No matching parameters." }
                    } else {
                        ul { class: "menu bg-base-100 rounded-box w-full p-1 border border-base-300 max-h-64 overflow-y-auto",
                            for p in suggestions {
                                li { key: "{p.key}",
                                    button {
                                        r#type: "button",
                                        class: "flex flex-col items-start gap-0",
                                        onclick: move |_| {
                                            // Booleans default to Enabled (a valid
                                            // select value); others start empty.
                                            let value = if p.kind == InputKind::Bool {
                                                "true".to_string()
                                            } else {
                                                String::new()
                                            };
                                            rows.write().push(Row { key: p.key, value, error: None });
                                            query.set(String::new());
                                        },
                                        span { class: "font-medium text-sm", "{p.label}" }
                                        span { class: "text-xs text-base-content/50", "{p.desc}" }
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Active overrides (the inputs you set), nearest the Save bar ──
                div { class: "space-y-3 pt-2 border-t border-base-300",
                    if current_rows.is_empty() {
                        p { class: "text-base-content/50 text-sm",
                            "No overrides set yet — search above to add one."
                        }
                    }
                    for row in current_rows.iter() {
                        OverrideRow { key: "{row.key}", rows, row: row.clone() }
                    }
                }

                if let Some(err) = server_error() {
                    div { class: "alert alert-error text-sm", "{err}" }
                }

                // ── Actions — "Clear all" left, the Save (submit) pulled right. ──
                div { class: "flex flex-wrap items-center justify-between gap-2 pt-2",
                    button {
                        class: "btn btn-error btn-outline",
                        r#type: "button",
                        disabled: mutate_disabled || (clear.busy)(),
                        onclick: move |_| {
                            server_error.set(None);
                            clear.open();
                        },
                        "Clear all"
                    }
                    button {
                        class: "btn btn-primary",
                        r#type: "button",
                        disabled: mutate_disabled || (save.busy)(),
                        onclick: move |_| {
                            // Validate the working set; on success stash the built
                            // data and open the confirm modal.
                            server_error.set(None);
                            let mut data = ConfigOverridesData::default();
                            let mut working = rows.read().clone();
                            let mut ok = true;
                            for r in working.iter_mut() {
                                match apply_value(&mut data, r.key, &r.value) {
                                    Ok(()) => r.error = None,
                                    Err(e) => {
                                        r.error = Some(e);
                                        ok = false;
                                    }
                                }
                            }
                            rows.set(working);
                            if ok {
                                pending.set(Some(data));
                                save.open();
                            }
                        },
                        "Save overrides"
                    }
                }
                }
            }
            }

            // ── Save confirmation ─────────────────────────────────────────────
            if (save.open)() {
                ConfirmModal {
                    title: "Save config overrides?",
                    body: "This replaces the server's overrides file with the current set. Restart the server afterwards to apply them.",
                    confirm_label: "Save",
                    busy: (save.busy)(),
                    title_id: "confirm-save-overrides-title",
                    on_cancel: move |_| save.close(),
                    on_confirm: move |_| {
                        let Some(data) = pending.peek().clone() else { return };
                        save.run(
                            cfg,
                            |c| async move { c.set_config_overrides(data).await },
                            move |res| match res {
                                Ok(_) => {
                                    toast.success(
                                        "Overrides saved — restart the server for them to take effect.",
                                    );
                                    nav.replace(Route::ViewConfig {});
                                }
                                Err(e) => server_error.set(Some(e)),
                            },
                        );
                    },
                }
            }

            // ── Clear-all confirmation ────────────────────────────────────────
            if (clear.open)() {
                ConfirmModal {
                    title: "Clear all overrides?",
                    body: "This deletes every override and reverts the server to its configured defaults. Restart afterwards to apply.",
                    confirm_label: "Clear all",
                    danger: true,
                    busy: (clear.busy)(),
                    title_id: "confirm-clear-overrides-title",
                    on_cancel: move |_| clear.close(),
                    on_confirm: move |_| {
                        clear.run(
                            cfg,
                            |c| async move { c.delete_config_overrides().await },
                            move |res| match res {
                                Ok(()) => {
                                    toast.success(
                                        "All overrides cleared — restart the server to apply.",
                                    );
                                    nav.replace(Route::ViewConfig {});
                                }
                                Err(e) => server_error.set(Some(e)),
                            },
                        );
                    },
                }
            }
        }
    }
}
