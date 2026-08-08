//! Playback / download preference value types persisted in
//! [`ClientConfig`](super::ClientConfig).

use serde::{Deserialize, Serialize};

/// Shared `<select>` value mapping for the small string-keyed preference enums.
///
/// Each enum supplies only its variant table (`ALL`) and the stable serde token
/// (`as_str`); the round-trip parse (`from_str_or_default`) is provided once here —
/// an unknown token falls back to the enum's `Default`. Callers that invoke
/// `Enum::from_str_or_default(s)` must have this trait in scope.
pub trait SelectEnum: Sized + Default + Copy + 'static {
    /// All variants, in `<select>` display order.
    const ALL: &'static [Self];
    /// Stable identifier (serde name) used as the `<option>` value.
    fn as_str(&self) -> &'static str;
    /// The variant whose `as_str` matches `s`, or the default for an unknown token.
    fn from_str_or_default(s: &str) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|p| p.as_str() == s)
            .unwrap_or_default()
    }
}

/// Declare a small string-keyed `<select>` enum and its boilerplate in one place:
/// the unit enum (with the standard derives serde needs), the inherent `ALL`
/// variant table, `as_str` (stable serde token) / `label` (human display), and the
/// [`SelectEnum`] impl. Mark the default with `#[default]` on its variant; attach
/// extra methods (e.g. `bytes`, `percent`) in a separate `impl` block. Keeping the
/// table and the two match arms generated from one variant list means they can't
/// drift out of sync.
macro_rules! select_enum {
    (
        $(#[$emeta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident => ($token:literal, $label:literal)
            ),+ $(,)?
        }
    ) => {
        $(#[$emeta])*
        #[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
        $vis enum $name {
            $(
                $(#[$vmeta])*
                $variant,
            )+
        }

        impl $name {
            /// All variants, in `<select>` display order.
            pub const ALL: [Self; [$(stringify!($variant)),+].len()] = [$(Self::$variant),+];

            /// Stable identifier (serde name) for `<select>` values.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $token,)+
                }
            }

            /// Human label for the settings select.
            pub fn label(&self) -> &'static str {
                match self {
                    $(Self::$variant => $label,)+
                }
            }
        }

        impl SelectEnum for $name {
            const ALL: &'static [Self] = &$name::ALL;
            fn as_str(&self) -> &'static str {
                $name::as_str(self)
            }
        }
    };
}

select_enum! {
    /// Client-side UI font size scaling factor.
    ///
    /// Applied as a CSS `font-size` percentage on the root element; all `rem`-based
    /// sizing (text, spacing, padding) scales proportionally.
    pub enum FontSize {
        /// Base size (1×).
        Small => ("Small", "Small (100%)"),
        /// 25 % larger than small (1.25×).
        #[default]
        Medium => ("Medium", "Medium (125%, default)"),
        /// 50 % larger than small (1.5×).
        Large => ("Large", "Large (150%)"),
        /// 75 % larger than small (1.75×).
        XLarge => ("XLarge", "X-Large (175%)"),
    }
}

impl FontSize {
    /// CSS `font-size` percentage applied to `<html>`. Everything in the app is
    /// sized in `rem`, so scaling the root font-size scales text, buttons,
    /// inputs, and spacing together.
    pub fn percent(&self) -> &'static str {
        match self {
            FontSize::Small => "100%",
            FontSize::Medium => "125%",
            FontSize::Large => "150%",
            FontSize::XLarge => "175%",
        }
    }
}

select_enum! {
    /// How the play button sources audio — the app is local-first, so the default
    /// never streams.
    ///
    /// Governs PLAY behavior only: the download badge and context-menu download
    /// actions always work manually, and a device copy plays offline in every mode.
    pub enum PlaybackPreference {
        /// Play local bytes only. No device copy → download first (spinner), then
        /// play. Never streams; "Stream from server" is hidden.
        #[default]
        DownloadOnly => ("DownloadOnly", "Download only (local-first)"),
        /// Stream the server copy immediately AND download to the device in the
        /// background; later plays hit the local bytes.
        StreamFirstAndDownload => (
            "StreamFirstAndDownload",
            "Stream first, download in background"
        ),
        /// Local-first; stream the server copy only when there's no device copy.
        /// Play never triggers downloads.
        StreamFallback => ("StreamFallback", "Local first, stream as fallback"),
        /// Always stream; play never touches device storage.
        StreamOnly => ("StreamOnly", "Stream only"),
    }
}

select_enum! {
    /// Per-request byte size for device downloads. A device download pulls the
    /// server's audio copy in HTTP `Range` chunks so a dropped connection only
    /// re-fetches the chunk it lost; this picks how big each chunk is. `NoChunking`
    /// fetches the whole file in a single request (no `Range` windowing) — simplest,
    /// but buffers the entire body in memory.
    pub enum DownloadChunkSize {
        TwoMB => ("TwoMB", "2 MB"),
        #[default]
        FourMB => ("FourMB", "4 MB (default)"),
        EightMB => ("EightMB", "8 MB"),
        SixteenMB => ("SixteenMB", "16 MB"),
        ThirtyTwoMB => ("ThirtyTwoMB", "32 MB"),
        NoChunking => ("NoChunking", "No chunking (whole file)"),
    }
}

impl DownloadChunkSize {
    /// Chunk size in bytes, or `None` for `NoChunking` (fetch the whole file in
    /// one request).
    pub fn bytes(&self) -> Option<u64> {
        let mb = match self {
            DownloadChunkSize::TwoMB => 2,
            DownloadChunkSize::FourMB => 4,
            DownloadChunkSize::EightMB => 8,
            DownloadChunkSize::SixteenMB => 16,
            DownloadChunkSize::ThirtyTwoMB => 32,
            DownloadChunkSize::NoChunking => return None,
        };
        Some(mb * 1024 * 1024)
    }
}

/// Selectable parallel-chunk counts for a single device download — how many
/// chunks are fetched concurrently (writes still land in order). `1` = the
/// historical sequential download.
pub const DOWNLOAD_PARALLELISMS: [u8; 4] = [1, 2, 4, 8];

fn default_download_parallelism() -> u8 {
    1
}

/// Client-side device-download preferences: chunk size and per-download
/// parallelism. Local-only, persisted in `ClientConfig`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct DownloadPrefs {
    /// Per-request chunk size (default 4 MB; `NoChunking` for a single request).
    #[serde(default)]
    pub chunk_size: DownloadChunkSize,
    /// Concurrent chunk fetches within one download (default 1 = sequential).
    #[serde(default = "default_download_parallelism")]
    pub parallelism: u8,
}

impl Default for DownloadPrefs {
    fn default() -> Self {
        Self {
            chunk_size: DownloadChunkSize::default(),
            parallelism: default_download_parallelism(),
        }
    }
}

/// Client-side playback preferences.
/// These are client-only settings that affect playback behavior.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlaybackPrefs {
    /// How the play button sources audio (default: DownloadOnly — never stream).
    #[serde(default)]
    pub playback_preference: PlaybackPreference,
    /// Skip forward interval in seconds (default: 30).
    #[serde(default = "default_skip_forward")]
    pub skip_forward: i32,
    /// Skip backward interval in seconds (default: 15).
    #[serde(default = "default_skip_backward")]
    pub skip_backward: i32,
    /// Default playback rate (default: 1.0).
    #[serde(default = "default_playback_rate")]
    pub playback_rate: f32,
    /// Auto-advance to next queue item when episode ends (default: true).
    #[serde(default = "default_auto_advance")]
    pub auto_advance: bool,
    /// Add episodes to the FRONT of the queue (position 0) instead of the end
    /// (default: true → newest first). Only affects the queue (the default
    /// playlist).
    #[serde(default = "default_add_to_queue_front")]
    pub add_to_queue_front: bool,
    /// Default sleep-timer duration in minutes — the value used when the sleep
    /// timer is activated (tapped, or auto-armed via `sleep_by_default`).
    #[serde(default = "default_sleep_minutes")]
    pub default_sleep_minutes: i32,
    /// Step (minutes) the sleep timer is nudged by on the player's +/− controls.
    #[serde(default = "default_sleep_increment")]
    pub sleep_increment_minutes: i32,
    /// Auto-arm the sleep timer (with `default_sleep_minutes`) once when playback
    /// starts. The timer then spans the whole listening session — it is not
    /// re-armed per episode (default: false).
    #[serde(default)]
    pub sleep_by_default: bool,
    /// Media-control override: some Bluetooth devices/headsets only expose
    /// "next/previous track" buttons (no seek). When set, those buttons skip
    /// forward/backward within the current episode by the skip intervals
    /// instead (default: false).
    #[serde(default)]
    pub media_next_prev_seek: bool,
}

impl Default for PlaybackPrefs {
    fn default() -> Self {
        Self {
            playback_preference: PlaybackPreference::default(),
            skip_forward: default_skip_forward(),
            skip_backward: default_skip_backward(),
            playback_rate: default_playback_rate(),
            auto_advance: default_auto_advance(),
            add_to_queue_front: default_add_to_queue_front(),
            default_sleep_minutes: default_sleep_minutes(),
            sleep_increment_minutes: default_sleep_increment(),
            sleep_by_default: false,
            media_next_prev_seek: false,
        }
    }
}

fn default_skip_forward() -> i32 {
    30
}
fn default_skip_backward() -> i32 {
    15
}
fn default_playback_rate() -> f32 {
    1.0
}
fn default_add_to_queue_front() -> bool {
    true
}

/// Selectable playback rates, shared by the settings prefs control and the
/// now-playing speed picker so both offer the same set (every configured rate,
/// e.g. 1.25×, is selectable in the now-playing picker).
pub const PLAYBACK_RATES: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
fn default_auto_advance() -> bool {
    true
}
fn default_sleep_minutes() -> i32 {
    30
}
fn default_sleep_increment() -> i32 {
    5
}

/// Selectable default sleep-timer durations (minutes) for the settings picker.
pub const SLEEP_DURATIONS: [i32; 9] = [5, 10, 15, 20, 30, 45, 60, 90, 120];
/// Selectable sleep-timer increments (minutes) for the +/− step setting.
pub const SLEEP_INCREMENTS: [i32; 4] = [5, 10, 15, 30];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_chunk_size_default_and_bytes() {
        // Default is 4 MB; `bytes()` reflects the labelled sizes; no-chunking is None.
        assert_eq!(DownloadChunkSize::default(), DownloadChunkSize::FourMB);
        assert_eq!(DownloadChunkSize::TwoMB.bytes(), Some(2 * 1024 * 1024));
        assert_eq!(DownloadChunkSize::FourMB.bytes(), Some(4 * 1024 * 1024));
        assert_eq!(
            DownloadChunkSize::ThirtyTwoMB.bytes(),
            Some(32 * 1024 * 1024)
        );
        assert_eq!(DownloadChunkSize::NoChunking.bytes(), None);
    }

    #[test]
    fn download_chunk_size_str_roundtrip() {
        for c in DownloadChunkSize::ALL {
            assert_eq!(DownloadChunkSize::from_str_or_default(c.as_str()), c);
        }
        // Unknown token falls back to the default.
        assert_eq!(
            DownloadChunkSize::from_str_or_default("bogus"),
            DownloadChunkSize::default()
        );
    }

    #[test]
    fn download_prefs_default_is_four_mb_sequential() {
        let dp = DownloadPrefs::default();
        assert_eq!(dp.chunk_size, DownloadChunkSize::FourMB);
        assert_eq!(dp.parallelism, 1);
    }
}
