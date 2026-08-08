//! Media Session API action handlers (web only).
//!
//! Routes hardware/OS media controls (Bluetooth headsets, lock screens, the
//! browser media hub) through the [`PlayerController`] so playback state stays
//! in sync. [`register`] returns a [`MediaSessionHandlers`] guard the
//! `PlayerProvider` holds for its lifetime (dropped on unmount, which clears the
//! browser handlers) — so a per-user-switch remount can't leak closures. Every
//! callback peeks the live client config, so settings changes apply immediately
//! without re-registering.
//!
//! The interesting part is the next/previous-track override: many Bluetooth
//! devices only expose "next/previous track" (no seek). With
//! `PlaybackPrefs::media_next_prev_seek` set, those actions skip
//! forward/backward within the current episode by the configured skip
//! intervals instead of changing track. Without it (the default) they drive
//! the real next/previous-episode navigation (queue continuation, then
//! podcast order).
//!
//! The browser invokes these closures from its own event loop with NO Dioxus
//! runtime on the thread-local stack, so each handler is wrapped in a
//! [`ScopeBound`] captured at registration: it re-enters the registering
//! provider's runtime + scope SYNCHRONOUSLY around the body (a track-change
//! reaches `dioxus::prelude::spawn`, which panics without a current scope —
//! an unrecoverable wasm freeze). Synchronous, not a channel pump like the
//! webview variant, because the body must stay on the browser's
//! user-activation call stack for autoplay policy (see `scope_bound.rs`).
//!
//! Bound by hand instead of via web-sys: web-sys gates the whole Media Session
//! API behind `--cfg=web_sys_unstable_apis`, and threading an extra rustflag
//! through every build path (dx, cargo, CI) is more fragile than these few
//! lines of binding.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use super::{PlayerController, ScopeBound};
use halogen_ui_config::ClientConfig;

#[wasm_bindgen]
extern "C" {
    /// `navigator.mediaSession`.
    type MediaSession;

    /// `catch`: some browsers throw `TypeError` for action names they don't
    /// know; an unsupported action should be skipped, not panic the app.
    #[wasm_bindgen(method, catch, js_name = setActionHandler)]
    fn set_action_handler(
        this: &MediaSession,
        action: &str,
        handler: Option<&js_sys::Function>,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(method, setter, js_name = metadata)]
    fn set_metadata(this: &MediaSession, metadata: &JsValue);

    #[wasm_bindgen(method, setter, js_name = playbackState)]
    fn set_playback_state(this: &MediaSession, state: &str);

    /// `catch`: throws when position > duration or values are non-finite.
    #[wasm_bindgen(method, catch, js_name = setPositionState)]
    fn set_position_state(this: &MediaSession, state: &JsValue) -> Result<(), JsValue>;

    /// Global `MediaMetadata` constructor. `catch`: ReferenceError on
    /// browsers without the API.
    type MediaMetadata;

    #[wasm_bindgen(constructor, catch)]
    fn new(init: &JsValue) -> Result<MediaMetadata, JsValue>;
}

/// `navigator.mediaSession`, or `None` where the API is unsupported.
fn session() -> Option<MediaSession> {
    let window = web_sys::window()?;
    match js_sys::Reflect::get(
        window.navigator().as_ref(),
        &JsValue::from_str("mediaSession"),
    ) {
        Ok(v) if !v.is_undefined() && !v.is_null() => Some(v.unchecked_into()),
        _ => None,
    }
}

fn set_prop(obj: &js_sys::Object, key: &str, value: &JsValue) {
    js_sys::Reflect::set(obj, &JsValue::from_str(key), value).ok();
}

/// Lock-screen / car-display metadata for the current episode. Podcast
/// convention: episode title as the track title, podcast name as both artist
/// and album (players show whichever field their layout has room for).
pub fn update_metadata(title: &str, podcast: &str, artwork_url: Option<&str>) {
    let Some(session) = session() else { return };
    let init = js_sys::Object::new();
    set_prop(&init, "title", &JsValue::from_str(title));
    set_prop(&init, "artist", &JsValue::from_str(podcast));
    set_prop(&init, "album", &JsValue::from_str(podcast));
    if let Some(url) = artwork_url {
        let art = js_sys::Object::new();
        set_prop(&art, "src", &JsValue::from_str(url));
        set_prop(&init, "artwork", js_sys::Array::of1(art.as_ref()).as_ref());
    }
    if let Ok(metadata) = MediaMetadata::new(init.as_ref()) {
        session.set_metadata(metadata.as_ref());
    }
}

/// Clear the OS "now playing" surface (playback stopped).
pub fn clear_metadata() {
    if let Some(session) = session() {
        session.set_metadata(&JsValue::NULL);
        session.set_playback_state("none");
    }
}

/// Reflect play/pause into the OS controls (they don't observe the `<audio>`
/// element's state — the app drives it through the controller).
pub fn update_playback_state(playing: bool) {
    if let Some(session) = session() {
        session.set_playback_state(if playing { "playing" } else { "paused" });
    }
}

/// Position/duration/rate → the lock-screen progress bar. The OS extrapolates
/// between updates from the rate, so per-tick refreshes are cheap insurance
/// for seeks, not a requirement.
pub fn update_position_state(position_secs: f64, duration_secs: Option<f64>, rate: f32) {
    let Some(duration) = duration_secs.filter(|d| d.is_finite() && *d > 0.0) else {
        return;
    };
    let Some(session) = session() else { return };
    let state = js_sys::Object::new();
    set_prop(&state, "duration", &JsValue::from_f64(duration));
    set_prop(
        &state,
        "position",
        &JsValue::from_f64(position_secs.clamp(0.0, duration)),
    );
    // playbackRate must be non-zero or some browsers throw.
    set_prop(
        &state,
        "playbackRate",
        &JsValue::from_f64(if rate > 0.0 { rate as f64 } else { 1.0 }),
    );
    session.set_position_state(state.as_ref()).ok();
}

/// Action names registered by [`register`], cleared on drop of
/// [`MediaSessionHandlers`]. Kept in sync with the `bind` calls below.
const ACTIONS: [&str; 6] = [
    "play",
    "pause",
    "seekforward",
    "seekbackward",
    "nexttrack",
    "previoustrack",
];

thread_local! {
    /// Bumped by each [`register`]; a guard clears the browser handlers on drop
    /// only while it is still the current session. On a user switch Dioxus builds
    /// the new provider's handlers (a fresh `register`, bumping this) BEFORE
    /// dropping the old one, so an unconditional clear in `Drop` would wipe the
    /// handlers the new session just bound — leaving controls dead until reload.
    static SESSION_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Owns the Media Session action-handler closures for the lifetime of one
/// `PlayerProvider`. Replaces the old `Closure::forget()` page-lifetime leak:
/// `PlayerProvider` remounts per user-switch, so leaking a fresh closure set
/// (each cloning a `PlayerController`) on every remount accumulated forever. The
/// provider now holds this guard and drops it on unmount, which also clears the
/// browser handlers so a stale (dropped-closure) handler can't linger.
#[must_use = "dropping the handlers immediately unregisters the media controls"]
pub struct MediaSessionHandlers {
    /// This registration's generation; only clear handlers on drop if we're still
    /// the current one (no newer `register` has taken over).
    generation: u64,
    // Kept alive so the registered handlers stay callable; cleared on drop.
    _closures: Vec<Closure<dyn FnMut()>>,
}

impl Drop for MediaSessionHandlers {
    fn drop(&mut self) {
        // A newer session took over (bumped the generation past ours) → leave its
        // handlers bound; only the current session clears on teardown.
        if SESSION_GEN.with(|g| g.get()) != self.generation {
            return;
        }
        // Clear the browser handlers before the closures are freed, so the OS
        // never holds a pointer to a dropped closure.
        if let Some(session) = session() {
            for action in ACTIONS {
                session.set_action_handler(action, None).ok();
            }
        }
    }
}

/// Register all Media Session action handlers and RETURN them as a
/// [`MediaSessionHandlers`] guard. The caller (`PlayerProvider`) holds the guard
/// for its lifetime and drops it on unmount — no page-lifetime leak across
/// remounts. `None` where the API is unsupported.
pub fn register(
    controller: PlayerController,
    config: Signal<ClientConfig>,
) -> Option<MediaSessionHandlers> {
    let session = session()?;
    let generation = SESSION_GEN.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    });
    // Captured inside PlayerProvider's `use_hook` (a component scope is
    // guaranteed there); re-entered per browser invocation below.
    let scope = ScopeBound::capture();

    let mut closures: Vec<Closure<dyn FnMut()>> = Vec::with_capacity(ACTIONS.len());
    let mut bind = |action: &str, f: Box<dyn FnMut()>| {
        let mut on_action = scope.bind(f);
        let wrapped = move || {
            // Stale-registration no-op: once a newer `register` bumped the
            // generation (user switch), this provider's scope may already be
            // unmounted, and re-entering a removed scope panics the moment a
            // handler spawns. Checked OUTSIDE the re-entry so a stale
            // invocation never touches the dead scope at all.
            if SESSION_GEN.with(|g| g.get()) != generation {
                return;
            }
            on_action();
        };
        let closure = Closure::wrap(Box::new(wrapped) as Box<dyn FnMut()>);
        session
            .set_action_handler(action, Some(closure.as_ref().unchecked_ref()))
            .ok();
        closures.push(closure);
    };

    // Play/pause through the controller (not the raw element) so the
    // `now_playing` state and cursor persistence stay correct. `play` (not
    // `resume`): the OS play action can arrive in any state, and `resume` is a
    // no-op outside `Paused` — which left this button dead after a track ended or
    // a play failed, while `pause` kept working.
    {
        let c = controller.clone();
        bind("play", Box::new(move || c.play()));
    }
    {
        let c = controller.clone();
        bind("pause", Box::new(move || c.pause()));
    }

    // Devices that natively expose seek buttons: honor the skip intervals.
    {
        let c = controller.clone();
        bind(
            "seekforward",
            Box::new(move || {
                c.seek_relative(config.peek().playback_prefs.skip_forward as f64);
            }),
        );
    }
    {
        let c = controller.clone();
        bind(
            "seekbackward",
            Box::new(move || {
                c.seek_relative(-(config.peek().playback_prefs.skip_backward as f64));
            }),
        );
    }

    // Track-change buttons: with the override on, seek within the episode;
    // otherwise navigate the queue/podcast (the OS shows these buttons as enabled,
    // so a no-op would be a dead button — drive the real next/previous instead).
    {
        let c = controller.clone();
        bind(
            "nexttrack",
            Box::new(move || {
                let prefs = config.peek().playback_prefs.clone();
                if prefs.media_next_prev_seek {
                    c.seek_relative(prefs.skip_forward as f64);
                } else {
                    c.play_next_episode();
                }
            }),
        );
    }
    {
        let c = controller;
        bind(
            "previoustrack",
            Box::new(move || {
                let prefs = config.peek().playback_prefs.clone();
                if prefs.media_next_prev_seek {
                    c.seek_relative(-(prefs.skip_backward as f64));
                } else {
                    c.play_previous_episode();
                }
            }),
        );
    }

    Some(MediaSessionHandlers {
        generation,
        _closures: closures,
    })
}
