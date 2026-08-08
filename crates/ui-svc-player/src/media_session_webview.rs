//! Media Session integration for the native webview targets — the OS
//! lock-screen / hardware-key counterpart of the wasm `media_session` module,
//! with the same public surface (`register` + the `update_*`/`clear_metadata`
//! free functions) so `PlayerProvider` wires both targets identically.
//!
//! The webview engines are the same browser engines the web build runs in, so
//! `navigator.mediaSession` is the integration point here too — WebKitGTK,
//! WKWebView, and the Android WebView surface it to the OS where they support
//! it (GNOME MPR/media keys, iOS Now Playing, Android media notifications),
//! and the JS side feature-detects so an engine without the API degrades to a
//! silent no-op instead of breaking playback. Deeper per-OS integration (MPRIS
//! over D-Bus, `MPNowPlayingInfoCenter`, a foreground-service MediaSession)
//! can later replace this module without touching the provider, since the
//! surface matches the wasm module.
//!
//! Everything rides ONE persistent eval channel (`SESSION`):
//! - **Rust → JS**: metadata / playback-state / position updates via
//!   [`Eval::send`] — the position refresh ticks ~4×/sec, and a fresh
//!   `document::eval` per update would grow the query slab unboundedly, so
//!   the channel is created once in [`register`].
//! - **JS → Rust**: OS action events (`play`/`pause`/`seekforward`/…) via
//!   `dioxus.send`, pumped by a scope-bound task into [`PlayerController`]
//!   calls — including the Bluetooth next/previous→seek override
//!   (`media_next_prev_seek`), mirroring the wasm handlers.

use std::cell::RefCell;

use dioxus::prelude::document::Eval;
use dioxus::prelude::*;
use serde_json::json;

use super::PlayerController;
use halogen_ui_config::ClientConfig;

thread_local! {
    /// The live channel [`register`] opened, used by the `update_*` free
    /// functions. UI-thread only (like everything in this crate), so a
    /// thread-local is the right scope. `None` before register / after drop.
    static SESSION: RefCell<Option<Eval>> = const { RefCell::new(None) };
    /// Bumped by each [`register`]; a guard tears its session down on drop only
    /// while it is still the current one. On a user switch Dioxus builds the new
    /// provider's session (a fresh `register`, bumping this) BEFORE dropping the
    /// old one — an unconditional teardown would detach the channel and clear the
    /// OS handlers the new session just bound, killing lock-screen controls.
    static SESSION_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// JS side: feature-detect `navigator.mediaSession`, bind the action handlers
/// (each forwards to Rust), then apply update messages until teardown. The
/// `finally` clears the handlers so a dropped guard never leaves the OS
/// pointing at a dead channel.
const SESSION_JS: &str = r#"
const ACTIONS = ['play', 'pause', 'seekforward', 'seekbackward', 'nexttrack', 'previoustrack'];
const ms = ('mediaSession' in navigator) ? navigator.mediaSession : null;
if (ms) {
  for (const action of ACTIONS) {
    try { ms.setActionHandler(action, () => { try { dioxus.send({ action }); } catch (_) {} }); } catch (_) {}
  }
}
// Mark this generation as the current owner of the global handlers, so an older
// session's teardown (below) can tell it's been superseded and must NOT clear.
self.__hgMediaGen = __GEN__;
try {
  for (;;) {
    const m = await dioxus.recv();
    if (m.op === 'teardown') break;
    if (!ms) continue;
    if (m.op === 'metadata') {
      try { ms.metadata = new MediaMetadata(m.value); } catch (_) {}
    } else if (m.op === 'clear') {
      try { ms.metadata = null; ms.playbackState = 'none'; } catch (_) {}
    } else if (m.op === 'playback') {
      try { ms.playbackState = m.playing ? 'playing' : 'paused'; } catch (_) {}
    } else if (m.op === 'position') {
      try { ms.setPositionState({ duration: m.duration, position: m.position, playbackRate: m.rate }); } catch (_) {}
    }
  }
} finally {
  // Only clear if we're STILL the current session. On a user switch the new
  // session binds handlers + sets a higher `__hgMediaGen` before this old one
  // tears down, so clearing here would wipe the new session's handlers.
  if (ms && self.__hgMediaGen === __GEN__) {
    for (const action of ACTIONS) { try { ms.setActionHandler(action, null); } catch (_) {} }
    try { ms.metadata = null; ms.playbackState = 'none'; } catch (_) {}
  }
}
"#;

/// Send on the live session channel; silently a no-op before [`register`] /
/// after the guard dropped (mirrors the wasm module's "no API → no-op").
fn session_send(msg: serde_json::Value) {
    SESSION.with(|s| {
        if let Some(chan) = s.borrow().as_ref() {
            let _ = chan.send(msg);
        }
    });
}

/// Lock-screen / notification metadata for the current episode. Same field
/// convention as the wasm module: episode title as the track title, podcast
/// name as artist and album. `artwork_url` is the relative
/// `/halogen-media/...` proxy URL — the webview resolves it against the app
/// origin, so the OS artwork fetch rides the authenticated proxy too.
pub fn update_metadata(title: &str, podcast: &str, artwork_url: Option<&str>) {
    let mut value = json!({ "title": title, "artist": podcast, "album": podcast });
    if let Some(url) = artwork_url {
        value["artwork"] = json!([{ "src": url }]);
    }
    session_send(json!({ "op": "metadata", "value": value }));
}

/// Clear the OS "now playing" surface (playback stopped).
pub fn clear_metadata() {
    session_send(json!({ "op": "clear" }));
}

/// Reflect play/pause into the OS controls.
pub fn update_playback_state(playing: bool) {
    session_send(json!({ "op": "playback", "playing": playing }));
}

/// Position/duration/rate → the OS progress bar. Skipped without a finite
/// duration (some engines throw on position > duration / non-finite values —
/// the JS side also guards).
pub fn update_position_state(position_secs: f64, duration_secs: Option<f64>, rate: f32) {
    let Some(duration) = duration_secs.filter(|d| d.is_finite() && *d > 0.0) else {
        return;
    };
    session_send(json!({
        "op": "position",
        "duration": duration,
        "position": position_secs.clamp(0.0, duration),
        "rate": if rate > 0.0 { rate as f64 } else { 1.0 },
    }));
}

/// Owns the session channel for the lifetime of one `PlayerProvider`. Dropping
/// it tears the JS side down (handlers cleared, metadata wiped) and detaches
/// the `update_*` functions — so a per-user-switch remount can't stack
/// handlers or leak the channel.
#[must_use = "dropping the handlers immediately unregisters the media controls"]
pub struct MediaSessionHandlers {
    channel: Eval,
    /// This registration's generation; only detach the shared channel on drop if
    /// we're still the current session (no newer `register` has replaced it).
    generation: u64,
}

impl Drop for MediaSessionHandlers {
    fn drop(&mut self) {
        // Break THIS session's JS loop; the JS `finally` guards its own
        // handler-clearing by generation, so it won't touch a newer session's.
        let _ = self.channel.send(json!({ "op": "teardown" }));
        // Only detach the shared channel if we're still current — a newer session
        // already replaced it (and rebound the handlers), so don't null it out.
        if SESSION_GEN.with(|g| g.get()) == self.generation {
            SESSION.with(|s| {
                s.borrow_mut().take();
            });
        }
    }
}

/// Open the session channel, bind the OS action handlers, and route their
/// events into `controller`. Returns the guard `PlayerProvider` holds for its
/// lifetime. `Option` for signature parity with the wasm module (this arm
/// always returns `Some`; JS feature-detects instead).
pub fn register(
    controller: PlayerController,
    config: Signal<ClientConfig>,
) -> Option<MediaSessionHandlers> {
    let generation = SESSION_GEN.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    });
    // Interpolate our generation into the JS (`__GEN__` appears in the owner
    // stamp + the teardown guard), so the JS can tell whether it's still current.
    let js = SESSION_JS.replace("__GEN__", &generation.to_string());
    let channel = dioxus::prelude::document::eval(&js);
    SESSION.with(|s| *s.borrow_mut() = Some(channel));

    // Action pump: scope-bound to the caller (`PlayerProvider`), and exits on
    // its own once the channel errors after teardown.
    let mut pump_chan = channel;
    spawn(async move {
        while let Ok(msg) = pump_chan.recv::<serde_json::Value>().await {
            let Some(action) = msg.get("action").and_then(|a| a.as_str()) else {
                continue;
            };
            let prefs = config.peek().playback_prefs.clone();
            match action {
                // Play/pause through the controller (not the raw element) so
                // `now_playing` and cursor persistence stay correct. `play` (not
                // `resume`): the OS play action can arrive in any state, and
                // `resume` is a no-op outside `Paused` — which left this button
                // dead after a track ended or a play failed, while `pause` kept
                // working.
                "play" => controller.play(),
                "pause" => controller.pause(),
                "seekforward" => controller.seek_relative(prefs.skip_forward as f64),
                "seekbackward" => controller.seek_relative(-(prefs.skip_backward as f64)),
                // Track-change buttons honor the Bluetooth next/previous→seek
                // override, mirroring the wasm handlers.
                "nexttrack" => {
                    if prefs.media_next_prev_seek {
                        controller.seek_relative(prefs.skip_forward as f64);
                    } else {
                        controller.play_next_episode();
                    }
                }
                "previoustrack" => {
                    if prefs.media_next_prev_seek {
                        controller.seek_relative(-(prefs.skip_backward as f64));
                    } else {
                        controller.play_previous_episode();
                    }
                }
                _ => {}
            }
        }
    });

    Some(MediaSessionHandlers {
        channel,
        generation,
    })
}
