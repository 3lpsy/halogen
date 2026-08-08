//! Web audio backend: an `HtmlAudioElement` driven from the UI thread.
//!
//! Control methods (load/play/pause/seek/rate) operate directly on the element.
//! `poll_events` snapshots the element's clock and drains any queued events.
//! Media Session API metadata/handlers (OS lock-screen controls) live in the
//! sibling `media_session` module, not here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::HtmlAudioElement;

use super::{MediaSource, PlayerBackend, PlayerEvent};

/// A one-sample silent WAV, looped by [`PlayerBackend::prime`] to latch the
/// user gesture onto the audio element (see the `prime` impl). Inline data URI:
/// no network, no asset, decodes instantly.
const SILENT_WAV: &str =
    "data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQAAAAA=";

/// `HTMLMediaElement.readyState` value `HAVE_FUTURE_DATA`: below this there
/// isn't enough buffered to keep playing, so we report buffering.
const HAVE_FUTURE_DATA: u16 = 3;

/// `HTMLMediaElement.readyState` value `HAVE_METADATA`: duration + seekable
/// ranges are known, so a deferred resume seek can be applied reliably.
const HAVE_METADATA: u16 = 1;

/// `HTMLMediaElement.readyState` value `HAVE_CURRENT_DATA`: data for the
/// current position exists. Below this the element can't be rendering audio,
/// so a clock snapshot would be a resource-selection artifact, not playback.
const HAVE_CURRENT_DATA: u16 = 2;

/// Fallback MIME for a device copy whose stored blob type is somehow empty —
/// WebKit resolves `blob:` media strictly by type, and an empty `type` on the
/// `<source>` child is itself a `MEDIA_ERR_SRC_NOT_SUPPORTED`. Matches the
/// media store's own download-time default.
const DEFAULT_LOCAL_AUDIO_TYPE: &str = "audio/mpeg";

/// Detach every playback source from the element: the `src` attribute AND any
/// `<source>` children. Device-copy loads attach a typed `<source>` child (see
/// [`set_typed_source`]) while remote/primer loads set the `src` attribute —
/// the two must never coexist, because a present `src` attribute wins resource
/// selection and silently masks the child.
fn clear_sources(audio: &HtmlAudioElement) {
    audio.remove_attribute("src").ok();
    while let Ok(Some(child)) = audio.query_selector("source") {
        child.remove();
    }
}

/// Attach the device-copy source as a `<source src=… type=…>` child instead of
/// assigning the `src` attribute. Two WebKit reasons: blob media resolution is
/// strict about the MIME type, and the iOS 17.4.1 regression class broke
/// blob URLs assigned via `src` while the typed `<source>` form kept working
/// (the workaround an Apple media engineer recommended). No-DOM hosts degrade
/// to an empty element, whose load failure surfaces as a normal player error.
fn set_typed_source(audio: &HtmlAudioElement, url: &str, content_type: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(el) = doc.create_element("source") else {
        return;
    };
    el.set_attribute("src", url).ok();
    el.set_attribute("type", content_type).ok();
    audio.append_child(&el).ok();
}

/// Audio backend backed by a single `<audio>` element.
///
/// The element is created once up front. Creation only fails when there's no
/// DOM (e.g. a non-browser wasm host), in which case `audio` is `None`, every
/// control is a no-op, and the failure surfaces as a `PlayerEvent::Error`
/// rather than panicking the render.
pub struct WebPlayerBackend {
    audio: Option<HtmlAudioElement>,
    events: Rc<RefCell<Vec<PlayerEvent>>>,
    /// The `blob:` object URL currently loaded (device-copy playback). The
    /// backend owns its lifetime: minting is the media store's job, revoking
    /// the *displaced* URL on the next load is ours. Never revoke the active
    /// one — the element is still reading from it.
    object_url: RefCell<Option<String>>,
    /// A resume position to apply once metadata loads. Setting `currentTime`
    /// before `HAVE_METADATA` is unreliable (silently snaps to 0), so `load`
    /// stashes it here and `poll_events` applies it when the element is ready.
    pending_seek: RefCell<Option<f64>>,
    /// The element has been unlocked for programmatic playback — a `play()`
    /// happened under a user gesture (a real one, or the [`Self::prime`] silent
    /// primer). Once set, deferred plays (post-download) are allowed, so
    /// priming again is pointless.
    primed: Rc<Cell<bool>>,
    /// The silent primer is currently the element's source. Element-derived
    /// events (time/buffering/ended, the native pause/playing listeners) are
    /// suppressed while set — the primer must be invisible to the controller's
    /// state machine (its `playing`/`ended` are not the episode's).
    priming: Rc<Cell<bool>>,
    /// Native `pause`/`playing` event listeners on the element. The app mostly
    /// drives play state from its own commands, but the OS can pause/resume the
    /// element on its own (audio-route change when a Bluetooth / wired headset is
    /// swapped, an incoming call, another app taking the audio focus). Polling the
    /// `paused` flag once per tick misses a transition that happens-and-reverts
    /// between polls, leaving the controller stuck "Playing" while silent — so we
    /// observe the native events directly and queue [`PlayerEvent::Paused`] /
    /// [`PlayerEvent::Playing`] the instant they fire. Held here to keep the
    /// closures alive for the element's lifetime; dropping them detaches the
    /// listeners. `None` when there's no DOM (non-browser wasm host).
    _on_pause: Option<Closure<dyn FnMut()>>,
    _on_playing: Option<Closure<dyn FnMut()>>,
}

impl WebPlayerBackend {
    pub fn new() -> Self {
        let (audio, init) = match HtmlAudioElement::new() {
            Ok(a) => (Some(a), Vec::new()),
            Err(_) => (
                None,
                vec![PlayerEvent::Error(
                    "audio unavailable: could not create <audio> element".to_string(),
                )],
            ),
        };
        let events = Rc::new(RefCell::new(init));
        let primed = Rc::new(Cell::new(false));
        let priming = Rc::new(Cell::new(false));
        // Observe the element's native pause/resume so an OS-initiated transition
        // (headset swap, focus loss) is reflected the moment it happens, not
        // sampled — see the field docs. The `pause` listener skips the end-of-track
        // pause (the element flips `paused` true at `ended`; the `Ended` path owns
        // that transition). Both skip the silent primer entirely (see `priming`).
        let (on_pause, on_playing) = match &audio {
            Some(audio) => {
                let ev = events.clone();
                let el = audio.clone();
                let priming_flag = priming.clone();
                let on_pause = Closure::<dyn FnMut()>::new(move || {
                    if !el.ended() && !priming_flag.get() {
                        ev.borrow_mut().push(PlayerEvent::Paused);
                    }
                });
                let ev = events.clone();
                let priming_flag = priming.clone();
                let on_playing = Closure::<dyn FnMut()>::new(move || {
                    if !priming_flag.get() {
                        ev.borrow_mut().push(PlayerEvent::Playing);
                    }
                });
                audio
                    .add_event_listener_with_callback("pause", on_pause.as_ref().unchecked_ref())
                    .ok();
                audio
                    .add_event_listener_with_callback(
                        "playing",
                        on_playing.as_ref().unchecked_ref(),
                    )
                    .ok();
                (Some(on_pause), Some(on_playing))
            }
            None => (None, None),
        };
        Self {
            audio,
            events,
            object_url: RefCell::new(None),
            pending_seek: RefCell::new(None),
            primed,
            priming,
            _on_pause: on_pause,
            _on_playing: on_playing,
        }
    }

    /// Record a non-fatal backend failure for the controller to drain.
    fn push_error(&self, msg: String) {
        self.events.borrow_mut().push(PlayerEvent::Error(msg));
    }

    /// Track the active object URL, revoking the one it displaces.
    fn replace_object_url(&self, new: Option<String>) {
        if let Some(old) = self.object_url.borrow_mut().take() {
            web_sys::Url::revoke_object_url(&old).ok();
        }
        *self.object_url.borrow_mut() = new;
    }
}

impl Default for WebPlayerBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// The element outlives the struct unless silenced here: an audibly-playing
/// `HtmlAudioElement` is never collected (per the HTML spec), so dropping the
/// backend mid-playback — the account-switch/sign-out keyed remount is exactly
/// that — would leave a detached element playing with no surviving handle to
/// stop it. The native listeners must also come off before their `Closure`s
/// drop, or the zombie element's later `pause`/`playing` events would invoke
/// freed closures.
impl Drop for WebPlayerBackend {
    fn drop(&mut self) {
        let Some(audio) = &self.audio else { return };
        if let Some(cb) = &self._on_pause {
            audio
                .remove_event_listener_with_callback("pause", cb.as_ref().unchecked_ref())
                .ok();
        }
        if let Some(cb) = &self._on_playing {
            audio
                .remove_event_listener_with_callback("playing", cb.as_ref().unchecked_ref())
                .ok();
        }
        let _ = audio.pause();
        // Same spec-correct source release as `stop`: detach sources + `load()`
        // (never src="" — it resolves to the page URL and errors), then revoke
        // the active blob URL so the device-copy bytes aren't pinned for the
        // rest of the page lifetime.
        clear_sources(audio);
        audio.load();
        self.replace_object_url(None);
    }
}

impl PlayerBackend for WebPlayerBackend {
    fn load(&self, src: MediaSource, start_at_secs: f64) {
        let Some(audio) = &self.audio else { return };
        // A real source displaces any silent primer: re-enable element events
        // and drop the primer's loop flag.
        self.priming.set(false);
        audio.set_loop(false);
        match src {
            MediaSource::Local(local) => {
                // Same-origin blob: URL — no credentials involved. Loaded via a
                // typed <source> child (see `set_typed_source`). Take ownership
                // of the URL (revoke whatever it displaces).
                audio.set_cross_origin(None);
                clear_sources(audio);
                set_typed_source(
                    audio,
                    &local.url,
                    local
                        .content_type
                        .as_deref()
                        .unwrap_or(DEFAULT_LOCAL_AUDIO_TYPE),
                );
                self.replace_object_url(Some(local.url));
            }
            MediaSource::Remote(url) => {
                // Send the `auth_media` cookie with the media request and accept
                // the credentialed response — required for the cross-origin dev
                // setup (UI and API on different ports); harmless same-origin.
                audio.set_cross_origin(Some("use-credentials"));
                clear_sources(audio);
                audio.set_src(&url);
                self.replace_object_url(None);
            }
        }
        audio.load();
        // Defer the resume seek: `currentTime` set before metadata loads is
        // unreliable and silently snaps to 0. `poll_events` applies it once the
        // element reaches HAVE_METADATA.
        *self.pending_seek.borrow_mut() = (start_at_secs > 0.0).then_some(start_at_secs);
    }

    fn play(&self) {
        let Some(audio) = &self.audio else { return };
        match audio.play() {
            // `play()` resolves/rejects asynchronously. Autoplay-policy
            // (`NotAllowedError`) and decode rejections do NOT set the element's
            // `error`, so await the promise to catch them — but ignore
            // `AbortError`, the normal rejection when a new load/pause supersedes
            // this play (e.g. switching episodes quickly).
            Ok(promise) => {
                let events = self.events.clone();
                let primed = self.primed.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        // A successful play unlocks the element for future
                        // programmatic plays — no primer needed after this.
                        Ok(_) => primed.set(true),
                        Err(err) => {
                            let name = js_sys::Reflect::get(&err, &"name".into())
                                .ok()
                                .and_then(|v| v.as_string())
                                .unwrap_or_default();
                            if name != "AbortError" {
                                // Carry the DOMException name: NotAllowedError
                                // (autoplay policy) vs NotSupportedError (source
                                // failed to load) point at entirely different
                                // failures, and the device log is often all we
                                // get from an iOS report.
                                let msg = if name.is_empty() {
                                    "playback couldn't start".to_string()
                                } else {
                                    format!("playback couldn't start ({name})")
                                };
                                events.borrow_mut().push(PlayerEvent::Error(msg));
                            }
                        }
                    }
                });
            }
            Err(e) => self.push_error(format!("play failed: {e:?}")),
        }
    }

    fn pause(&self) {
        let Some(audio) = &self.audio else { return };
        if let Err(e) = audio.pause() {
            self.push_error(format!("pause failed: {e:?}"));
        }
    }

    fn seek(&self, secs: f64) {
        let Some(audio) = &self.audio else {
            *self.pending_seek.borrow_mut() = None;
            return;
        };
        // Metadata not loaded yet: `set_current_time` is unreliable here and
        // silently snaps to 0 (see `load`). Defer the target like `load` does so
        // `poll_events` applies it once the element reaches HAVE_METADATA, instead
        // of writing a clobbered position and losing the seek.
        if audio.ready_state() < HAVE_METADATA {
            *self.pending_seek.borrow_mut() = Some(secs);
            return;
        }
        audio.set_current_time(secs);
        // Metadata is loaded, so this seek lands now and supersedes any deferred
        // resume seek: clear it so `poll_events` doesn't snap playback back to the
        // resume cursor.
        *self.pending_seek.borrow_mut() = None;
    }

    fn set_rate(&self, rate: f32) {
        if let Some(audio) = &self.audio {
            audio.set_playback_rate(rate as f64);
        }
    }

    fn stop(&self) {
        let Some(audio) = &self.audio else { return };
        self.priming.set(false);
        audio.set_loop(false);
        let _ = audio.pause();
        audio.set_current_time(0.0);
        // Detach the sources and revoke the active blob URL so a teardown doesn't
        // pin the device-copy bytes until the next load. `stop` is a controller
        // teardown (next play re-loads), so this is safe. Removing the sources
        // + `load()` is the spec-correct release — never set src="" (it resolves
        // to the page URL and errors).
        clear_sources(audio);
        audio.load();
        self.replace_object_url(None);
        *self.pending_seek.borrow_mut() = None;
    }

    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        let mut out = std::mem::take(&mut *self.events.borrow_mut());
        let Some(audio) = &self.audio else { return out };
        // While the silent primer is loaded, its clock/buffering/ended snapshots
        // are meaningless to the controller (the episode is still Preparing) —
        // drain queued events only.
        if self.priming.get() {
            return out;
        }
        // Apply a deferred resume seek once metadata is loaded (see `load`).
        let pending = *self.pending_seek.borrow();
        if let Some(target) = pending
            && audio.ready_state() >= HAVE_METADATA
        {
            audio.set_current_time(target);
            *self.pending_seek.borrow_mut() = None;
        }
        // Snapshot the clock so the controller can persist the cursor — but withhold
        // it until a pending resume seek has landed, so the cursor isn't reported
        // (or persisted, or used to flip Loading→Playing) as 0 mid-resume. Also
        // withhold it until the element is actually progressing (`!paused` +
        // HAVE_CURRENT_DATA): `TimeUpdate` is the controller's Loading→Playing
        // signal, and an early snapshot during resource selection (cursor-0 loads
        // have no pending seek) would fake "playback started" — disarming the
        // loading-stall guard and the device-copy error retry, both of which only
        // act while `Loading`.
        if self.pending_seek.borrow().is_none()
            && !audio.paused()
            && audio.ready_state() >= HAVE_CURRENT_DATA
        {
            out.push(PlayerEvent::TimeUpdate(audio.current_time()));
        }
        // Duration becomes known once metadata loads.
        let dur = audio.duration();
        if dur.is_finite() && dur > 0.0 {
            out.push(PlayerEvent::DurationKnown(dur));
        }
        out.push(PlayerEvent::Buffering(
            audio.ready_state() < HAVE_FUTURE_DATA,
        ));
        if let Some(err) = audio.error() {
            // Include the platform's diagnostic message (WebKit's is often the
            // only clue distinguishing "bad bytes" from "couldn't read source").
            let detail = err.message();
            out.push(PlayerEvent::Error(if detail.is_empty() {
                format!("audio error (code {})", err.code())
            } else {
                format!("audio error (code {}: {detail})", err.code())
            }));
        }
        if audio.ended() {
            out.push(PlayerEvent::Ended);
            return out;
        }
        // OS-initiated pause/resume (audio-route change, incoming call, focus loss)
        // is surfaced by the native `pause`/`playing` listeners wired in `new`, not
        // sampled here — see the `_on_pause`/`_on_playing` field docs.
        out
    }

    /// Latch the current user gesture onto the audio element by playing a
    /// silent, looping data-URI source. Browsers (Chrome in particular) reject
    /// an un-gestured `audio.play()` with `NotAllowedError` until the element
    /// has played once under user activation — which broke Download & Play on a
    /// fresh page: the real play fires seconds later, when the download lands,
    /// with no activation left. Playing the primer *inside* the click unlocks
    /// the element; the real `load()` then displaces it. Loop + event
    /// suppression (see `priming`) keep the primer invisible to the controller
    /// (no `ended`, no `playing` flip while the episode shows Preparing).
    fn prime(&self) {
        let Some(audio) = &self.audio else { return };
        if self.primed.get() {
            return;
        }
        // Mid-playback the element is already unlocked (that play succeeded).
        if !audio.paused() {
            self.primed.set(true);
            return;
        }
        self.priming.set(true);
        audio.set_cross_origin(None);
        audio.set_loop(true);
        // The primer rides the `src` attribute; drop any typed <source> child a
        // device-copy load left behind so exactly one source form is present.
        clear_sources(audio);
        audio.set_src(SILENT_WAV);
        self.replace_object_url(None);
        *self.pending_seek.borrow_mut() = None;
        match audio.play() {
            Ok(promise) => {
                let primed = self.primed.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // Resolved → unlocked. Rejected (including AbortError from a
                    // real load displacing the primer before it starts) → leave
                    // `primed` false so the next gesture retries; the primer is
                    // best-effort and must never surface an error.
                    if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() {
                        primed.set(true);
                    }
                });
            }
            Err(_) => self.priming.set(false),
        }
    }
}
