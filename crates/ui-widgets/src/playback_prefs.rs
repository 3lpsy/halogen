//! The playback-preferences settings section: playback source, skip intervals,
//! default rate, and auto-advance. Named `PlaybackPrefsForm` (the `…Form`
//! vocabulary) to disambiguate from the `PlaybackPrefs` data struct it edits.

use crate::{CheckboxField, SelectField};
use dioxus::prelude::*;
use halogen_ui_config::{
    PLAYBACK_RATES, PlaybackPreference, PlaybackPrefs, SLEEP_DURATIONS, SLEEP_INCREMENTS,
    SelectEnum,
};

#[component]
pub fn PlaybackPrefsForm(
    prefs: Signal<PlaybackPrefs>,
    /// Embedded-server mode: the playback source is forced to Stream Only (the
    /// player's single chokepoint does the forcing) — the select shows that
    /// state, locked; the stored preference survives for a later switch back
    /// to a remote server.
    #[props(default)]
    embedded: bool,
) -> Element {
    let mut prefs = prefs;

    // Local mirrors of the two skip fields' text, so a transient empty/partial edit
    // (e.g. clearing the field before typing a new number) isn't snapped back
    // mid-keystroke. A parseable value is committed to `prefs`; an empty/invalid one
    // is held in the local string without fighting the user (B11). Seeded from the
    // current prefs; the bound `value` reads the local string from here on.
    let mut skip_forward_text = use_signal(|| prefs().skip_forward.to_string());
    let mut skip_backward_text = use_signal(|| prefs().skip_backward.to_string());

    // Resync the text mirrors when the parent replaces `prefs` after mount (async
    // load / reset-to-defaults) — otherwise these two inputs stay stale while every
    // other field reads `prefs()` live. Each effect subscribes to its own field via
    // a PartialEq memo slice — subscribing to the whole `prefs` would re-run on any
    // other pref's commit and refill a cleared/partial (unparseable) in-progress
    // edit. Only overwrite when the incoming value differs from what the text
    // already represents, so a parseable in-progress edit (which committed the same
    // value back into `prefs`) isn't clobbered mid-typing.
    let skip_forward_committed = use_memo(move || prefs().skip_forward);
    let skip_backward_committed = use_memo(move || prefs().skip_backward);
    use_effect(move || {
        let fwd = skip_forward_committed();
        if skip_forward_text.peek().parse::<i32>().ok() != Some(fwd) {
            skip_forward_text.set(fwd.to_string());
        }
    });
    use_effect(move || {
        let back = skip_backward_committed();
        if skip_backward_text.peek().parse::<i32>().ok() != Some(back) {
            skip_backward_text.set(back.to_string());
        }
    });

    rsx! {
        div { class: "space-y-6",
            // Playback source (local-first by default). Locked to Stream Only in
            // embedded mode — with the server on this device, streaming IS local
            // playback and a device copy would duplicate every byte.
            SelectField {
                label: "Playback Source",
                description: if embedded {
                    "Locked to Stream only while using the embedded server — audio plays straight from the built-in server's downloads."
                } else {
                    "How the play button gets audio. Download-only keeps playback fully local; streaming modes use the server copy."
                },
                aria_label: "Playback source",
                width_class: "w-40",
                disabled: embedded,
                value: if embedded {
                    PlaybackPreference::StreamOnly.as_str().to_string()
                } else {
                    prefs().playback_preference.as_str().to_string()
                },
                options: PlaybackPreference::ALL
                    .iter()
                    .map(|p| (p.as_str().to_string(), p.label().to_string()))
                    .collect::<Vec<_>>(),
                onchange: move |v: String| {
                    prefs.set(PlaybackPrefs {
                        playback_preference: PlaybackPreference::from_str_or_default(&v),
                        ..prefs()
                    });
                },
            }
            // Skip intervals
            div { class: "space-y-2",
                h3 { class: "text-lg font-medium", "Skip Intervals" }
                div { class: "flex flex-wrap items-center justify-between gap-2",
                    span { class: "text-muted", "Skip forward (seconds)" }
                    input {
                        "aria-label": "Skip forward (seconds)",
                        class: "input input-bordered w-24",
                        r#type: "number",
                        value: "{skip_forward_text}",
                        oninput: move |e| {
                            skip_forward_text.set(e.value());
                            if let Ok(value) = e.value().parse::<i32>() {
                                prefs.set(PlaybackPrefs {
                                    skip_forward: value,
                                    ..prefs()
                                });
                            }
                        },
                    }
                }
                div { class: "flex flex-wrap items-center justify-between gap-2",
                    span { class: "text-muted", "Skip backward (seconds)" }
                    input {
                        "aria-label": "Skip backward (seconds)",
                        class: "input input-bordered w-24",
                        r#type: "number",
                        value: "{skip_backward_text}",
                        oninput: move |e| {
                            skip_backward_text.set(e.value());
                            if let Ok(value) = e.value().parse::<i32>() {
                                prefs.set(PlaybackPrefs {
                                    skip_backward: value,
                                    ..prefs()
                                });
                            }
                        },
                    }
                }
                // Media-control override for devices without seek buttons.
                CheckboxField {
                    checked: prefs().media_next_prev_seek,
                    label: "Next/previous track buttons skip within the episode (for Bluetooth devices without seek buttons)",
                    onchange: move |v| {
                        prefs.set(PlaybackPrefs {
                            media_next_prev_seek: v,
                            ..prefs()
                        });
                    },
                }
            }
            // Playback rate
            SelectField {
                label: "Default Playback Rate",
                aria_label: "Default playback rate",
                width_class: "w-32",
                value: format!("{:?}", prefs().playback_rate),
                options: PLAYBACK_RATES
                    .iter()
                    .map(|r| (format!("{r:?}"), format!("{r}x")))
                    .collect::<Vec<_>>(),
                onchange: move |v: String| {
                    if let Ok(rate) = v.parse::<f32>() {
                        prefs.set(PlaybackPrefs {
                            playback_rate: rate,
                            ..prefs()
                        });
                    }
                },
            }
            // Auto advance
            div { class: "space-y-2",
                h3 { class: "text-lg font-medium", "Auto-advance" }
                CheckboxField {
                    checked: prefs().auto_advance,
                    label: "Automatically play next episode in queue",
                    onchange: move |v| {
                        prefs.set(PlaybackPrefs { auto_advance: v, ..prefs() });
                    },
                }
            }
            div { class: "space-y-2",
                h3 { class: "text-lg font-medium", "Queue order" }
                CheckboxField {
                    checked: prefs().add_to_queue_front,
                    label: "Add episodes to the beginning of the queue",
                    onchange: move |v| {
                        prefs.set(PlaybackPrefs {
                            add_to_queue_front: v,
                            ..prefs()
                        });
                    },
                }
            }
            // Sleep timer — default duration, the +/- step used by the player's
            // sleep control, and whether playback auto-arms it.
            SelectField {
                label: "Default sleep timer",
                aria_label: "Default sleep timer (minutes)",
                width_class: "w-32",
                value: prefs().default_sleep_minutes.to_string(),
                options: SLEEP_DURATIONS
                    .iter()
                    .map(|d| (d.to_string(), format!("{d} min")))
                    .collect::<Vec<_>>(),
                onchange: move |v: String| {
                    if let Ok(minutes) = v.parse::<i32>() {
                        prefs.set(PlaybackPrefs {
                            default_sleep_minutes: minutes,
                            ..prefs()
                        });
                    }
                },
            }
            SelectField {
                label: "Sleep timer increment",
                aria_label: "Sleep timer increment (minutes)",
                width_class: "w-32",
                value: prefs().sleep_increment_minutes.to_string(),
                options: SLEEP_INCREMENTS
                    .iter()
                    .map(|d| (d.to_string(), format!("{d} min")))
                    .collect::<Vec<_>>(),
                onchange: move |v: String| {
                    if let Ok(minutes) = v.parse::<i32>() {
                        prefs.set(PlaybackPrefs {
                            sleep_increment_minutes: minutes,
                            ..prefs()
                        });
                    }
                },
            }
            div { class: "space-y-2",
                h3 { class: "text-lg font-medium", "Sleep by default" }
                CheckboxField {
                    checked: prefs().sleep_by_default,
                    label: "Start the sleep timer automatically when playback begins",
                    onchange: move |v| {
                        prefs.set(PlaybackPrefs {
                            sleep_by_default: v,
                            ..prefs()
                        });
                    },
                }
            }
        }
    }
}
