//! The overridable-parameter registry + value (de)serialization for the config
//! overrides editor. Pure domain logic (no Dioxus), kept apart from the component
//! so the param table and the parsers stay testable and readable.

use halogen_wire::ConfigOverridesData;

/// The input a parameter renders as.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum InputKind {
    /// Free whole number.
    Number,
    /// Bounded percentage (0–100).
    Percent,
    /// Boolean → a select (Enabled / Disabled).
    Bool,
    /// `YYYY-MM-DD` date.
    Date,
    /// Free text (a server filesystem path).
    Text,
}

/// Static metadata for one overridable parameter. `key` mirrors the
/// [`ConfigOverridesData`] field name exactly (it's the get/set lookup key).
pub(super) struct ParamMeta {
    pub key: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub kind: InputKind,
}

/// The full overridable allowlist (mirrors `ConfigOverridesData`). Order here is
/// the order shown in the picker.
pub(super) const PARAMS: &[ParamMeta] = &[
    ParamMeta {
        key: "subscription_fallback_poll_interval_secs",
        label: "Fallback poll interval (secs)",
        desc: "Default feed poll cadence",
        kind: InputKind::Number,
    },
    ParamMeta {
        key: "subscription_poll_wake_interval_secs",
        label: "Poll wake interval (secs)",
        desc: "How often the poller wakes",
        kind: InputKind::Number,
    },
    ParamMeta {
        key: "subscription_fallback_max_episodes",
        label: "Fallback max episodes",
        desc: "Max episodes fetched per feed",
        kind: InputKind::Number,
    },
    ParamMeta {
        key: "subscription_max_concurrent_downloads",
        label: "Max concurrent downloads",
        desc: "Parallel episode downloads",
        kind: InputKind::Number,
    },
    ParamMeta {
        key: "subscription_max_poll_concurrent",
        label: "Max concurrent polls",
        desc: "Parallel feed polls",
        kind: InputKind::Number,
    },
    ParamMeta {
        key: "subscription_poll_auto_download_enabled",
        label: "Auto-download on poll",
        desc: "Server-side auto-download default",
        kind: InputKind::Bool,
    },
    ParamMeta {
        key: "subscription_auto_playlist_add_to_start",
        label: "Auto-add to start of playlists",
        desc: "Insert auto-added episodes at the start",
        kind: InputKind::Bool,
    },
    ParamMeta {
        key: "subscription_no_sync_before",
        label: "No sync before",
        desc: "Ignore episodes before this date",
        kind: InputKind::Date,
    },
    ParamMeta {
        key: "subscription_sync_on_start",
        label: "Sync on start",
        desc: "Sync feeds at server boot",
        kind: InputKind::Bool,
    },
    ParamMeta {
        key: "auth_token_expiry_minutes",
        label: "Token expiry (mins)",
        desc: "JWT lifetime in minutes",
        kind: InputKind::Number,
    },
    ParamMeta {
        key: "episode_playback_complete_percentage",
        label: "Playback complete %",
        desc: "Mark finished within the last N%",
        kind: InputKind::Percent,
    },
    ParamMeta {
        key: "opml_file",
        label: "OPML file",
        desc: "Server path to a seed OPML file",
        kind: InputKind::Text,
    },
];

pub(super) fn meta_for(key: &str) -> Option<&'static ParamMeta> {
    PARAMS.iter().find(|p| p.key == key)
}

/// The current (set) value of `key` in `data`, stringified, or `None` if unset.
/// Booleans stringify to "true"/"false" — the select option values.
pub(super) fn current_value(data: &ConfigOverridesData, key: &str) -> Option<String> {
    match key {
        "subscription_fallback_poll_interval_secs" => data
            .subscription_fallback_poll_interval_secs
            .map(|v| v.to_string()),
        "subscription_poll_wake_interval_secs" => data
            .subscription_poll_wake_interval_secs
            .map(|v| v.to_string()),
        "subscription_fallback_max_episodes" => data
            .subscription_fallback_max_episodes
            .map(|v| v.to_string()),
        "subscription_max_concurrent_downloads" => data
            .subscription_max_concurrent_downloads
            .map(|v| v.to_string()),
        "subscription_max_poll_concurrent" => {
            data.subscription_max_poll_concurrent.map(|v| v.to_string())
        }
        "subscription_poll_auto_download_enabled" => data
            .subscription_poll_auto_download_enabled
            .map(|v| v.to_string()),
        "subscription_auto_playlist_add_to_start" => data
            .subscription_auto_playlist_add_to_start
            .map(|v| v.to_string()),
        "subscription_no_sync_before" => data.subscription_no_sync_before.clone(),
        "subscription_sync_on_start" => data.subscription_sync_on_start.map(|v| v.to_string()),
        "auth_token_expiry_minutes" => data.auth_token_expiry_minutes.map(|v| v.to_string()),
        "episode_playback_complete_percentage" => data
            .episode_playback_complete_percentage
            .map(|v| v.to_string()),
        "opml_file" => data.opml_file.clone(),
        _ => None,
    }
}

/// Parse `raw` and set it into `data` under `key`. Returns a short message on a
/// parse/range error (shown inline under the field).
pub(super) fn apply_value(
    data: &mut ConfigOverridesData,
    key: &str,
    raw: &str,
) -> Result<(), String> {
    let raw = raw.trim();
    match key {
        "subscription_fallback_poll_interval_secs" => {
            data.subscription_fallback_poll_interval_secs = Some(parse_u64(raw)?);
        }
        "subscription_poll_wake_interval_secs" => {
            data.subscription_poll_wake_interval_secs = Some(parse_u64(raw)?);
        }
        "subscription_fallback_max_episodes" => {
            data.subscription_fallback_max_episodes = Some(parse_usize(raw)?);
        }
        "subscription_max_concurrent_downloads" => {
            data.subscription_max_concurrent_downloads = Some(parse_usize(raw)?);
        }
        "subscription_max_poll_concurrent" => {
            data.subscription_max_poll_concurrent = Some(parse_usize(raw)?);
        }
        "subscription_poll_auto_download_enabled" => {
            data.subscription_poll_auto_download_enabled = Some(parse_bool(raw)?);
        }
        "subscription_auto_playlist_add_to_start" => {
            data.subscription_auto_playlist_add_to_start = Some(parse_bool(raw)?);
        }
        "subscription_no_sync_before" => {
            data.subscription_no_sync_before = Some(parse_date(raw)?);
        }
        "subscription_sync_on_start" => {
            data.subscription_sync_on_start = Some(parse_bool(raw)?);
        }
        "auth_token_expiry_minutes" => {
            data.auth_token_expiry_minutes = Some(parse_u64(raw)?);
        }
        "episode_playback_complete_percentage" => {
            let v = parse_u64(raw)?;
            if v > 100 {
                return Err("Must be between 0 and 100".to_string());
            }
            data.episode_playback_complete_percentage = Some(v as u16);
        }
        "opml_file" => {
            if raw.is_empty() {
                return Err("Enter a path".to_string());
            }
            data.opml_file = Some(raw.to_string());
        }
        _ => return Err("Unknown parameter".to_string()),
    }
    Ok(())
}

fn parse_u64(raw: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|_| "Enter a whole number".to_string())
}
fn parse_usize(raw: &str) -> Result<usize, String> {
    raw.parse::<usize>()
        .map_err(|_| "Enter a whole number".to_string())
}
fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("Choose enabled or disabled".to_string()),
    }
}
/// Lenient `YYYY-MM-DD` shape check (server re-parses and ignores malformed dates).
fn parse_date(raw: &str) -> Result<String, String> {
    let ok = raw.len() == 10
        && raw.as_bytes().iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if ok {
        Ok(raw.to_string())
    } else {
        Err("Use YYYY-MM-DD".to_string())
    }
}
