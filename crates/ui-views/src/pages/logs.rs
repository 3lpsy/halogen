use dioxus::prelude::*;

use crate::Route;
use crate::components::BackButton;
use halogen_ui_config::DeviceLogConfig;
use halogen_ui_config::config_actions::persist_config;
use halogen_ui_icons::{ClockRotateLeft, DocumentText, Download, Trash};
// (`use_config` below serves both viewers: device-log capture prefs and the
// server-log fetch client.)
use halogen_ui_logging::{self as logging, Level, LogLine};
use halogen_ui_state::hooks::use_config;

/// Client logs root (`/logs`). It has no content of its own (the real viewer is
/// the device-log page), so redirect there rather than landing on a blank page.
#[component]
pub fn Logs() -> Element {
    let nav = use_navigator();
    use_effect(move || {
        nav.replace(Route::DeviceLogs {});
    });
    rsx! {}
}

/// Server logs page (`/admin/logs`, admin-only).
///
/// Fetches a tail of the server's log file via `GET /admin/server-logs` and
/// renders it newest-first in the same code-block style as the device-log
/// viewer, with a manual refresh and a client-side search filter. Online-only
/// (the logs live on the server).
#[component]
pub fn ServerLogs() -> Element {
    let config = use_config();
    let mut query = use_signal(String::new);
    // Bumping this refetches the tail.
    let mut refresh = use_signal(|| 0u32);

    let logs = use_resource(move || {
        let _ = refresh();
        // `api_client_or_err` is None when unconfigured OR manually offline —
        // "Go Offline" suppresses the fetch like every other online-only surface.
        let client = config.read().api_client_or_err();
        async move {
            let client = client?;
            client
                .get_server_logs(None)
                .await
                .map_err(|e| e.to_string())
        }
    });

    rsx! {
        div { class: "p-2 space-y-4",
            BackButton {}
            div {
                h1 { class: "text-3xl font-bold", "Server Logs" }
                div { class: "flex items-center justify-between gap-2 mt-1",
                    p { class: "text-sm text-muted",
                        match &*logs.read_unchecked() {
                            Some(Ok(data)) => match &data.path {
                                Some(path) => format!("{} • {} lines", path, data.lines.len()),
                                // No log file → the server serves its in-memory
                                // ring (this process's lines only).
                                None => format!(
                                    "In-memory (no log file configured) • {} lines",
                                    data.lines.len()
                                ),
                            },
                            Some(Err(_)) => String::new(),
                            None => "Loading…".to_string(),
                        }
                    }
                    button {
                        class: "btn btn-ghost btn-sm btn-square",
                        "aria-label": "Refresh",
                        title: "Refresh",
                        onclick: move |_| refresh += 1,
                        ClockRotateLeft { class: "w-4 h-4" }
                    }
                }
            }

            input {
                "aria-label": "Search logs",
                class: "input input-bordered w-full text-sm",
                r#type: "text",
                placeholder: "Search logs…",
                value: "{query}",
                oninput: move |e| query.set(e.value()),
            }

            match &*logs.read_unchecked() {
                None => rsx! {
                    div { class: "flex justify-center py-16",
                        span { class: "loading loading-spinner" }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "flex flex-col items-center justify-center text-center gap-3 py-16 text-muted",
                        DocumentText { class: "w-12 h-12 opacity-60" }
                        p { class: "text-sm max-w-sm text-error", "Couldn't load server logs: {e}" }
                    }
                },
                Some(Ok(data)) => {
                    // Newest first, filtered client-side (the tail is bounded, so
                    // an in-memory filter is fine).
                    let q = query.read().trim().to_lowercase();
                    let shown: Vec<String> = data
                        .lines
                        .iter()
                        .rev()
                        .filter(|l| q.is_empty() || l.to_lowercase().contains(&q))
                        .cloned()
                        .collect();
                    rsx! {
                        div { class: "bg-base-300 border border-base-200 rounded-lg p-3 overflow-auto max-h-[70vh] font-mono text-xs leading-relaxed",
                            if shown.is_empty() {
                                p { class: "text-muted",
                                    if q.is_empty() {
                                        "No server log lines available."
                                    } else {
                                        "No logs match your search."
                                    }
                                }
                            } else {
                                for (i, line) in shown.iter().enumerate() {
                                    div {
                                        key: "{i}",
                                        class: "whitespace-pre-wrap break-words py-0.5",
                                        "{line}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Device logs viewer (`/logs/device`). Reads the in-app device-log ring
/// (`halogen_ui_logging`), newest line first. Manual refresh (no live tail), a
/// search filter, and a download button. Capture is gated by the "Enable
/// Device Logs" toggle on this page; when off, nothing new is recorded.
#[component]
pub fn DeviceLogs() -> Element {
    let config = use_config();
    let mut device_logs = use_signal(|| config().device_logs.clone());
    let mut query = use_signal(String::new);
    let mut lines = use_signal(Vec::<LogLine>::new);
    // Bumping this re-reads the (non-reactive) global ring.
    let mut refresh = use_signal(|| 0u32);
    // Transient note for the download result (native file path).
    let mut note = use_signal(|| Option::<String>::None);

    // Persist device-log settings AND apply them to the live logger so capture
    // changes take effect immediately (no reload).
    use_effect(move || {
        let dl = device_logs.read().clone();
        logging::set_enabled(dl.enabled);
        logging::set_level(dl.level);
        persist_config(config, move |c| c.device_logs = dl);
    });

    let enabled = device_logs().enabled;

    // Re-read whenever the query changes or refresh is bumped. Newest first.
    use_effect(move || {
        let _ = refresh();
        let q = query();
        let mut data = if q.trim().is_empty() {
            logging::snapshot()
        } else {
            logging::search(&q)
        };
        data.reverse();
        lines.set(data);
    });

    let count = lines.read().len();

    rsx! {
        div {
            class: "p-2 space-y-4",
            BackButton {}
            // Header: title, then the capture status with the Refresh / Download /
            // Clear actions as inline icon buttons on the same line.
            div {
                h1 { class: "text-3xl font-bold", "Device Logs" }
                div {
                    class: "flex items-center justify-between gap-2 mt-1",
                    p {
                        class: "text-sm text-muted",
                        if enabled {
                            "Capturing • {count} lines"
                        } else {
                            "Capture disabled"
                        }
                    }
                    div {
                        class: "flex items-center gap-1",
                        button {
                            class: "btn btn-ghost btn-sm btn-square",
                            "aria-label": "Refresh",
                            title: "Refresh",
                            onclick: move |_| {
                                note.set(None);
                                refresh.set(refresh() + 1);
                            },
                            ClockRotateLeft { class: "w-4 h-4" }
                        }
                        button {
                            class: "btn btn-ghost btn-sm btn-square",
                            "aria-label": "Download",
                            title: "Download",
                            onclick: move |_| {
                                let text = logging::export_text();
                                match logging::download_logs(&text) {
                                    Some(path) => note.set(Some(format!("Saved to {path}"))),
                                    None => note.set(Some("Download started".into())),
                                }
                            },
                            Download { class: "w-4 h-4" }
                        }
                        button {
                            class: "btn btn-ghost btn-sm btn-square",
                            "aria-label": "Clear",
                            title: "Clear",
                            onclick: move |_| async move {
                                // Drop the live ring + the not-yet-flushed queue, then
                                // wipe persisted storage so cleared lines don't return.
                                logging::clear();
                                let _ = logging::drain_pending();
                                logging::store::clear().await;
                                note.set(Some("Logs cleared".into()));
                                refresh.set(refresh() + 1);
                            },
                            Trash { class: "w-4 h-4" }
                        }
                    }
                }
            }

            // Capture controls — enable toggle + level threshold (moved here from
            // Settings so all device-log config lives with the viewer).
            div {
                class: "flex flex-wrap items-center gap-4 p-3 bg-base-200 rounded-lg",
                // Enable device logs
                div {
                    class: "flex items-center gap-2",
                    input {
                        r#type: "checkbox",
                        class: "checkbox",
                        checked: "{device_logs().enabled}",
                        onchange: move |e| {
                            device_logs.set(DeviceLogConfig {
                                enabled: e.checked(),
                                ..device_logs()
                            });
                        },
                    }
                    label { class: "text-sm", "Enable Device Logs" }
                },
                // Log level
                div {
                    class: "flex items-center gap-2",
                    label { class: "text-muted text-sm", "Log Level" }
                    select {
                        "aria-label": "Log level",
                        class: "select select-sm select-bordered",
                        value: "{device_logs().level.as_str()}",
                        onchange: move |e| {
                            device_logs.set(DeviceLogConfig {
                                level: Level::from_str_or_default(&e.value()),
                                ..device_logs()
                            });
                        },
                        for level in Level::ALL {
                            option {
                                value: "{level.as_str()}",
                                selected: "{device_logs().level == level}",
                                "{level.label()}"
                            }
                        }
                    }
                }
            }

            // Search
            input {
                "aria-label": "Search logs",
                class: "input input-bordered w-full text-sm",
                r#type: "text",
                placeholder: "Search logs…",
                value: "{query}",
                oninput: move |e| query.set(e.value()),
            }

            if let Some(msg) = note() {
                p { class: "text-xs text-success", "{msg}" }
            }

            // Log list (newest first), styled as a scrollable code block.
            div {
                class: "bg-base-300 border border-base-200 rounded-lg p-3 overflow-auto max-h-[70vh] font-mono text-xs leading-relaxed",
                if count == 0 {
                    p {
                        class: "text-muted",
                        if query.read().trim().is_empty() {
                            "No logs captured yet."
                        } else {
                            "No logs match your search."
                        }
                    }
                } else {
                    for (i, line) in lines.read().iter().enumerate() {
                        div {
                            key: "{i}",
                            class: "whitespace-pre-wrap break-words py-0.5 {line.level.color()}",
                            span { class: "text-muted", "{line.time_str()} " }
                            span { class: "font-semibold", "{line.level.as_str()} " }
                            span { class: "text-muted", "{line.target}: " }
                            "{line.msg}"
                        }
                    }
                }
            }
        }
    }
}
