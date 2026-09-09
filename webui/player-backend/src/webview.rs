//! Native audio uses a webview `Audio` element for OS decoding and playback rate. Eval messages carry controls and
//! queued events, including 250ms clock updates. The loopback bridge serves local files with Range support and proxies
//! authenticated remote media. Wry permits autoplay, so no gesture primer is needed.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use dioxus::prelude::*;
use serde_json::json;

use super::{MediaSource, PlayerBackend, PlayerEvent};
use halogen_webui_logging::warn;

/// Path segment for local-file serving on the loopback media server in
/// `ui-state::providers::webview_media`. Kept here, next to the URL producer,
/// and re-used by the server's route so the two can't drift.
pub const LOCAL_AUDIO_HANDLER: &str = "halogen-local-audio";

/// Path segment of the authenticated server-media proxy on the loopback media
/// server (also emitted by `ui-appstate::media_url::media_base` — string
/// literal there to keep that crate dependency-free).
pub const MEDIA_PROXY_HANDLER: &str = "halogen-media";

/// The JS side: one `Audio` element + event listeners + the command loop. Runs as an `AsyncFunction(dioxus)` so
/// top-level `await dioxus.recv()` works. A global teardown hook lets the *next* boot (provider remount on account
/// switch) silence a stale instance whose Rust side vanished without a `shutdown` (the pending `recv()` promise would
/// otherwise never settle).
const BOOT_JS: &str = r#"
if (window.__halogenAudioTeardown) { try { window.__halogenAudioTeardown(); } catch (_) {} }
const a = new Audio();
a.preload = 'auto';
let pendingSeek = null;
let torndown = false;
const send = (m) => { if (!torndown) { try { dioxus.send(m); } catch (_) {} } };
// Clock ticks only while playback actually progresses (not paused + data for
// the current position): the Rust controller treats the first 'time' after a
// load as "playback started" (Loading -> Playing), so a tick during resource
// selection would fake the start and disarm its loading-stall guard.
const tick = setInterval(() => {
  if (!a.currentSrc) return;
  if (pendingSeek === null && !a.paused && a.readyState >= 2) send({ t: 'time', v: a.currentTime });
}, 250);
const teardown = () => {
  if (torndown) return;
  torndown = true;
  clearInterval(tick);
  try { a.pause(); a.removeAttribute('src'); a.load(); } catch (_) {}
};
window.__halogenAudioTeardown = teardown;
a.addEventListener('loadedmetadata', () => {
  if (pendingSeek !== null) { try { a.currentTime = pendingSeek; } catch (_) {} pendingSeek = null; }
});
a.addEventListener('durationchange', () => {
  if (isFinite(a.duration) && a.duration > 0) send({ t: 'duration', v: a.duration });
});
a.addEventListener('pause', () => { if (!a.ended) send({ t: 'paused' }); });
a.addEventListener('playing', () => send({ t: 'playing' }));
a.addEventListener('ended', () => send({ t: 'ended' }));
a.addEventListener('waiting', () => send({ t: 'buffering', v: true }));
a.addEventListener('canplay', () => send({ t: 'buffering', v: false }));
a.addEventListener('error', () => {
  const code = a.error ? a.error.code : 0;
  send({ t: 'error', v: 'audio error (code ' + code + ')' });
});
try {
  for (;;) {
    const m = await dioxus.recv();
    if (m.cmd === 'load') {
      a.src = m.src;
      a.load();
      pendingSeek = m.startAt > 0 ? m.startAt : null;
    } else if (m.cmd === 'play') {
      const p = a.play();
      if (p && p.catch) {
        p.catch((e) => {
          if (!e || e.name !== 'AbortError') send({ t: 'error', v: "playback couldn't start" });
        });
      }
    } else if (m.cmd === 'pause') {
      a.pause();
    } else if (m.cmd === 'seek') {
      if (a.readyState < 1) { pendingSeek = m.secs; }
      else { try { a.currentTime = m.secs; } catch (_) {} pendingSeek = null; }
    } else if (m.cmd === 'rate') {
      a.playbackRate = m.rate;
    } else if (m.cmd === 'stop') {
      a.pause();
      try { a.currentTime = 0; } catch (_) {}
      a.removeAttribute('src');
      a.load();
      pendingSeek = null;
    } else if (m.cmd === 'shutdown') {
      break;
    }
  }
} finally {
  teardown();
}
"#;

/// Audio backend backed by an `<audio>` element in the app webview.
///
/// Must be constructed in a component/hook context (it spawns its event pump
/// on the current scope — `PlayerProvider`, whose lifetime it shares).
pub struct WebviewPlayerBackend {
    events: Rc<RefCell<Vec<PlayerEvent>>>,
    channel: dioxus::prelude::document::Eval,
}

impl WebviewPlayerBackend {
    pub fn new() -> Self {
        let channel = dioxus::prelude::document::eval(BOOT_JS);
        let events: Rc<RefCell<Vec<PlayerEvent>>> = Rc::new(RefCell::new(Vec::new()));

        // Pump JS events into the poll queue. Scope-bound (`spawn`): dropped
        // with the provider, and exits on its own when the JS loop returns
        // (shutdown) or the channel errors.
        let pump_events = events.clone();
        let mut pump_chan = channel;
        spawn(async move {
            while let Ok(msg) = pump_chan.recv::<serde_json::Value>().await {
                if let Some(ev) = decode_event(&msg) {
                    pump_events.borrow_mut().push(ev);
                }
            }
        });

        Self { events, channel }
    }

    fn send(&self, msg: serde_json::Value) {
        if let Err(e) = self.channel.send(msg) {
            // The webview channel died (teardown race). The stall guard will
            // surface a user-facing error if playback was in flight.
            warn!("webview audio channel send failed: {e:?}");
        }
    }
}

impl Default for WebviewPlayerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WebviewPlayerBackend {
    fn drop(&mut self) {
        // Explicit shutdown: lets the JS loop exit (and silence the element)
        // instead of hanging forever on a `recv()` whose Rust side is gone.
        let _ = self.channel.send(json!({ "cmd": "shutdown" }));
    }
}

/// Map one JS event message to a [`PlayerEvent`]; `None` for junk.
fn decode_event(msg: &serde_json::Value) -> Option<PlayerEvent> {
    let kind = msg.get("t")?.as_str()?;
    let ev = match kind {
        "time" => PlayerEvent::TimeUpdate(msg.get("v")?.as_f64()?),
        "duration" => PlayerEvent::DurationKnown(msg.get("v")?.as_f64()?),
        "buffering" => PlayerEvent::Buffering(msg.get("v")?.as_bool()?),
        "paused" => PlayerEvent::Paused,
        "playing" => PlayerEvent::Playing,
        "ended" => PlayerEvent::Ended,
        "error" => PlayerEvent::Error(msg.get("v")?.as_str()?.to_string()),
        _ => return None,
    };
    Some(ev)
}

/// Map a device file to `{local media base}/halogen-local-audio/{file}`. Webview audio requires HTTP(S); return `None`
/// for an unusable filename or unbound bridge so playback reports an error.
fn local_audio_url(path: &str) -> Option<String> {
    let name = Path::new(path).file_name()?.to_str()?;
    let base = halogen_webui_app_state::media_url::local_media_base()?;
    Some(format!("{base}/{LOCAL_AUDIO_HANDLER}/{name}"))
}

impl PlayerBackend for WebviewPlayerBackend {
    fn load(&self, src: MediaSource, start_at_secs: f64) {
        let url = match src {
            MediaSource::Local(local) => match local_audio_url(&local.url) {
                Some(url) => url,
                None => {
                    warn!("unusable device audio path: {}", local.url);
                    self.events.borrow_mut().push(PlayerEvent::Error(
                        "device copy path is unusable".to_string(),
                    ));
                    return;
                }
            },
            // Already a relative `/halogen-media/...` proxy URL on native (see
            // `ui-appstate::media_url::media_base`).
            MediaSource::Remote(url) => url,
        };
        self.send(json!({ "cmd": "load", "src": url, "startAt": start_at_secs }));
    }

    fn play(&self) {
        self.send(json!({ "cmd": "play" }));
    }

    fn pause(&self) {
        self.send(json!({ "cmd": "pause" }));
    }

    fn seek(&self, secs: f64) {
        self.send(json!({ "cmd": "seek", "secs": secs }));
    }

    fn set_rate(&self, rate: f32) {
        self.send(json!({ "cmd": "rate", "rate": rate }));
    }

    fn stop(&self) {
        self.send(json!({ "cmd": "stop" }));
    }

    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_audio_url_maps_file_name_under_local_base() {
        // No bridge bound (fresh process) → no URL, honest failure.
        assert_eq!(local_audio_url("/data/halogen/audio/42.mp3"), None);

        // Bound → absolute loopback URL with the FILE NAME only (never the
        // directory structure). Safe to set here: nextest runs each test in
        // its own process, so the OnceLock can't leak between tests.
        halogen_webui_app_state::media_url::set_local_media_base(
            "http://127.0.0.1:12345/test-nonce".to_string(),
        );
        assert_eq!(
            local_audio_url("/data/halogen/audio/42.mp3"),
            Some("http://127.0.0.1:12345/test-nonce/halogen-local-audio/42.mp3".to_string())
        );
    }

    #[test]
    fn decode_event_maps_known_kinds_and_rejects_junk() {
        assert!(matches!(
            decode_event(&serde_json::json!({"t": "time", "v": 3.5})),
            Some(PlayerEvent::TimeUpdate(v)) if v == 3.5
        ));
        assert!(matches!(
            decode_event(&serde_json::json!({"t": "buffering", "v": true})),
            Some(PlayerEvent::Buffering(true))
        ));
        assert!(matches!(
            decode_event(&serde_json::json!({"t": "ended"})),
            Some(PlayerEvent::Ended)
        ));
        assert!(decode_event(&serde_json::json!({"t": "nope"})).is_none());
        assert!(decode_event(&serde_json::json!({"v": 1.0})).is_none());
        // Wrong payload type for a known kind is junk, not a panic.
        assert!(decode_event(&serde_json::json!({"t": "time", "v": "NaN"})).is_none());
    }
}
