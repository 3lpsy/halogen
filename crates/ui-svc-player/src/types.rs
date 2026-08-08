//! Shared player value types: the playback `MediaSource`, `PlaybackState`,
//! `NowPlaying`, the `PlayerBackend` trait, and `PlayerEvent`. The controller
//! state machine + per-target backends live in their own modules.

use halogen_ui_svc_media::LocalAudio;

// ============================================================================
// Player Types
// ============================================================================

/// Source for audio playback. Never the origin feed URL.
///
/// The inner value is read by the web + webview backends; the renderless
/// native `NoopPlayerBackend` ignores it, so `dead_code` is allowed for that
/// build.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum MediaSource {
    /// The device's stored copy, as resolved by the media store. Web: a `blob:`
    /// object URL + the blob's MIME type — the backend takes ownership of the
    /// URL (revoking it once displaced) and loads it through a `<source type=…>`
    /// child, since WebKit resolves `blob:` media strictly by type. Native:
    /// `url` is an absolute file path (the webview backend maps it to the
    /// `halogen-local-audio` loopback media-server URL).
    Local(LocalAudio),
    /// The server's media endpoint (`/episodes/{id}/audio`). Web: absolute on
    /// the server, authenticated by the `auth_media` cookie. Native: a
    /// relative `/halogen-media/...` URL through the authenticated webview
    /// proxy (see `ui-appstate::media_url::media_base`).
    Remote(String),
}

/// Current playback state
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackState {
    /// A device download is in flight for the episode the user asked to play;
    /// the `PlayerProvider` watcher starts playback once the client download
    /// state hits `Downloaded` (bytes really on device). The Play control shows
    /// a spinner in this state.
    Preparing,
    /// Source set, waiting for the backend to start playing.
    Loading,
    Playing,
    Paused,
    Ended,
    Error(String),
}

/// The playback continuation context: the playlist the user pressed play from
/// (`None` = queue semantics). Consulted by auto-advance, the transport
/// Next/Prev, and the "Up next" surfaces so playing from a playlist continues
/// through *that* playlist. A newtype (not a bare `Option<i32>`) so it gets its
/// own Dioxus context slot; session-only — never persisted. Kept OUT of
/// [`NowPlaying`]: that struct is rebuilt on every track change, while the
/// context must survive auto-advance to the next episode in the playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayContext(pub Option<i32>);

/// Current player state
#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub episode_id: i32,
    pub state: PlaybackState,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub rate: f32,
    pub buffering: bool,
}

/// The minimal `now_playing` projection an episode row needs: which episode the
/// player is on and its coarse state. Rows derive `is_current`/`is_playing`/
/// `is_preparing` from this alone — they never read `position_secs`.
///
/// Used as the value of a `PartialEq`-gated `Memo` so the controller's ~4×/sec
/// position writes (which leave `episode_id` + `state` unchanged) recompute only
/// that one projection memo instead of invalidating every visible row's row-state
/// memo. See `PlayerProvider` / `use_now_playing_identity`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayingIdentity {
    pub episode_id: i32,
    pub state: PlaybackState,
}

impl From<&NowPlaying> for PlayingIdentity {
    fn from(n: &NowPlaying) -> Self {
        Self {
            episode_id: n.episode_id,
            state: n.state.clone(),
        }
    }
}

impl NowPlaying {
    /// `Loading` an episode at `start_at` (the resume cursor), carrying the user's
    /// playback `rate`. Buffering until the first frame.
    pub fn loading(episode_id: i32, start_at: f64, rate: f32) -> Self {
        Self {
            episode_id,
            state: PlaybackState::Loading,
            position_secs: start_at,
            duration_secs: None,
            rate,
            buffering: true,
        }
    }

    /// `Preparing` (spinner) while a device download runs before playback.
    pub fn preparing(episode_id: i32) -> Self {
        Self {
            episode_id,
            state: PlaybackState::Preparing,
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: true,
        }
    }

    /// A terminal `Error` state carrying the user-facing message.
    pub fn error(episode_id: i32, msg: impl Into<String>) -> Self {
        Self {
            episode_id,
            state: PlaybackState::Error(msg.into()),
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: false,
        }
    }

    /// Playback progress as a 0–100 percentage (0 when the duration is unknown).
    /// The one canonical home for the progress-bar math.
    pub fn progress_pct(&self) -> f64 {
        match self.duration_secs {
            Some(dur) if dur > 0.0 => (self.position_secs / dur * 100.0).clamp(0.0, 100.0),
            _ => 0.0,
        }
    }
}

// ============================================================================
// Player Backend Trait
// ============================================================================

/// Platform audio backend. Not `Send`/`Sync`: web backends hold JS handles and
/// everything runs on the UI thread (wasm single-threaded; native main thread).
pub trait PlayerBackend {
    fn load(&self, src: MediaSource, start_at_secs: f64);
    fn play(&self);
    fn pause(&self);
    fn seek(&self, secs: f64);
    fn set_rate(&self, rate: f32);
    fn stop(&self);
    fn poll_events(&mut self) -> Vec<PlayerEvent>;
    /// Unlock the backend for a `play()` that will happen OUTSIDE the current
    /// user gesture (the Download & Play path: the real play fires when the
    /// download lands, seconds after the click, where browser autoplay policy
    /// rejects it with `NotAllowedError` until the element has played once via a
    /// gesture). Must be called synchronously from the gesture's event handler.
    /// Default no-op: only backends with an autoplay policy (web) need it.
    fn prime(&self) {}
}

// ============================================================================
// Player Events
// ============================================================================

/// Events drained from a backend into the `now_playing` signal by the controller
/// (NOT `EpisodeState`). Constructed by the web (wasm) and webview (native
/// desktop/mobile) backends; the renderless native `NoopPlayerBackend` emits
/// nothing, so `dead_code` is allowed for that build.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PlayerEvent {
    /// The playback clock, and only while playback is actually progressing
    /// (element not paused + data for the current position). This is a
    /// CONTRACT, not a courtesy: the controller flips `Loading → Playing` on
    /// the first `TimeUpdate` after a load, and its loading-stall guard and
    /// device-copy error retry both act only while `Loading` — a backend that
    /// ticks during resource selection fakes the start and disarms both.
    TimeUpdate(f64),
    DurationKnown(f64),
    Buffering(bool),
    /// The element entered the *paused* state on its own — i.e. the OS paused it,
    /// not the app. Mobile browsers do this on an audio-route change (a speaker /
    /// Bluetooth headset / wired headphones disconnect), an incoming call, or
    /// another app taking the audio focus. The controller reflects it so the UI
    /// and the OS lock-screen controls match what's actually audible, instead of
    /// staying stuck "Playing" while the element is silent.
    Paused,
    /// The element resumed playing on its own (the OS handed audio focus back).
    /// Mirror of [`PlayerEvent::Paused`]; reflected back into the play state.
    Playing,
    Ended,
    Error(String),
}
