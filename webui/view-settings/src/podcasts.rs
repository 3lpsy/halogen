//! Admin OPML export downloads subscriptions; import validates XML with the shared parser before upload and refreshes
//! the worker on success. File selection/download is web-only.

use dioxus::prelude::*;

use super::SettingsSubpage;
use halogen_webui_commands::actions as commands;
use halogen_webui_hooks::{use_config, use_dispatch, use_is_admin, use_toast};
use halogen_wire::OpmlImportData;

#[component]
pub fn SettingsPodcasts() -> Element {
    let config = use_config();
    let dispatch = use_dispatch();
    let toast = use_toast();
    let is_admin = use_is_admin();

    // Authed client from the current config; `Err` message suitable for a toast.
    let build_client = move || config.read().api_client_or_err();

    rsx! {
        SettingsSubpage { title: "Podcasts",
            // Same gate the old Settings page applied to this whole section (the
            // menu also hides the link for non-admins; this covers deep links).
            if is_admin() {
                div {
                    class: "space-y-4 p-3 bg-base-200 rounded-lg",
                    div {
                        // flex-wrap so at large UI font scales the buttons drop below the
                        // text instead of overflowing the row.
                        class: "flex flex-wrap items-center justify-between gap-2",
                        div {
                            label { class: "text-base-content", "Subscriptions (OPML)" }
                            p {
                                class: "text-xs text-muted",
                                "Export your podcasts to an OPML file, or import one to subscribe in bulk."
                            }
                        },
                        div {
                            class: "flex flex-wrap items-center gap-2",
                            // Export
                            button {
                                class: "btn btn-primary btn-sm",
                                onclick: move |_| async move {
                                    let client = match build_client() {
                                        Ok(c) => c,
                                        Err(e) => { toast.error(e); return; }
                                    };
                                    match client.export_opml().await {
                                        Ok(d) => {
                                            halogen_webui_logging::download_text("halogen-subscriptions.opml", &d.opml);
                                            toast.success("Exported subscriptions");
                                        }
                                        Err(e) => toast.error(format!("Export failed: {e}")),
                                    }
                                },
                                "Export OPML"
                            },
                            // Import — hidden file input behind a button-styled label.
                            label {
                                class: "btn btn-primary btn-sm cursor-pointer",
                                "Import OPML"
                                input {
                                    r#type: "file",
                                    accept: ".opml,.xml,text/xml",
                                    class: "hidden",
                                    onchange: move |evt| async move {
                                        let files = evt.files();
                                        let Some(file) = files.into_iter().next() else { return };
                                        let xml = match file.read_string().await {
                                            Ok(s) => s,
                                            Err(_) => {
                                                toast.error("Could not read file");
                                                return;
                                            }
                                        };
                                        // Validate client-side before uploading.
                                        if let Err(e) = halogen_utils::opml::parse_opml_str(&xml) {
                                            toast.error(format!("Invalid OPML: {e}"));
                                            return;
                                        }
                                        let client = match build_client() {
                                            Ok(c) => c,
                                            Err(e) => { toast.error(e); return; }
                                        };
                                        match client.import_opml(OpmlImportData { opml: xml }).await {
                                            Ok(r) => {
                                                toast.success(format!(
                                                    "Imported {} podcast(s), {} skipped",
                                                    r.created, r.skipped
                                                ));
                                                commands::refresh(&dispatch);
                                            }
                                            Err(e) => toast.error(format!("Import failed: {e}")),
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            } else {
                p { class: "text-muted", "OPML import/export is admin-only." }
            }
        }
    }
}
