//! The download-preferences settings section: per-request chunk size and the
//! per-download parallel-chunk count. Named `DownloadPrefsForm` (the `…Form`
//! vocabulary) to disambiguate from the `DownloadPrefs` data struct it edits.

use crate::SelectField;
use dioxus::prelude::*;
use halogen_webui_config::{DOWNLOAD_PARALLELISMS, DownloadChunkSize, DownloadPrefs, SelectEnum};

#[component]
pub fn DownloadPrefsForm(prefs: Signal<DownloadPrefs>) -> Element {
    let mut prefs = prefs;
    let parallelism = prefs().parallelism;

    rsx! {
        div { class: "space-y-6",
            // Chunk size: how big each ranged request is (or no chunking at all).
            SelectField {
                label: "Download chunk size",
                description: "Bytes fetched per request when downloading to this device. Smaller chunks re-fetch less after a dropped connection; \"No chunking\" pulls the whole file in one request.",
                aria_label: "Download chunk size",
                width_class: "w-48",
                value: prefs().chunk_size.as_str().to_string(),
                options: DownloadChunkSize::ALL
                    .iter()
                    .map(|c| (c.as_str().to_string(), c.label().to_string()))
                    .collect::<Vec<_>>(),
                onchange: move |v: String| {
                    prefs.set(DownloadPrefs {
                        chunk_size: DownloadChunkSize::from_str_or_default(&v),
                        ..prefs()
                    });
                },
            }
            // Parallelism: concurrent chunk fetches within one download.
            SelectField {
                label: "Parallel chunks",
                description: "How many chunks to download at once. Higher values can be faster on high-latency connections but use more memory — roughly chunk size × parallel chunks held at once (e.g. 32 MB × 8 ≈ 256 MB).",
                aria_label: "Parallel chunks",
                width_class: "w-48",
                // No-chunking is a single request, so parallelism doesn't apply.
                disabled: prefs().chunk_size == DownloadChunkSize::NoChunking,
                value: parallelism.to_string(),
                options: DOWNLOAD_PARALLELISMS
                    .iter()
                    .map(|n| (n.to_string(), n.to_string()))
                    .collect::<Vec<_>>(),
                onchange: move |v: String| {
                    let n = v.parse::<u8>().unwrap_or(1);
                    prefs.set(DownloadPrefs { parallelism: n, ..prefs() });
                },
            }
        }
    }
}
