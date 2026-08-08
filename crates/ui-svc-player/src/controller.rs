//! The [`PlayerController`] — drives a [`PlayerBackend`] and reflects playback
//! into the dedicated `now_playing` signal. The controller lives here; `lib.rs` is
//! the crate shell, the player value types and the backend trait live in `types`,
//! and the no-op backend in `native`, so the playback state machine has its own
//! home.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;

use super::{
    MediaSource, NowPlaying, PlayContext, PlaybackState, PlayerBackend, PlayerEvent, SleepState,
    TICK_INTERVAL_MS, navigation, sleep,
};
// Aliased: this crate's own `PlaybackState` (super) is the player state machine
// (Playing/Paused/…); the worker-owned cursor overlay slice imports under a
// distinct name to avoid the clash.
use halogen_ui_appstate::PlaybackState as PlaybackOverlay;
use halogen_ui_appstate::{DownloadState, EpisodeState, PlaylistState};
use halogen_ui_commands::Command;
use halogen_ui_config::{ClientConfig, PlaybackPreference};
use halogen_ui_logging::{debug, info, warn};
use halogen_ui_svc_media::MediaStore;

/// Drives a [`PlayerBackend`] and reflects playback into a dedicated
/// `now_playing` signal (NOT `EpisodeState`).
///
/// Keeping player state out of `EpisodeState` is deliberate: the controller writes
/// `now_playing` ~4×/sec, and if that lived in `EpisodeState` every write would
/// re-render every `EpisodeState` subscriber (navbar, episode lists) — a render
/// storm while playing. `app_state` here is read-only (episode lookup + cursor).
/// Cheap to clone (everything is shared); UI-thread only (`Rc`, not `Send`).
#[derive(Clone)]
pub struct PlayerController {
    pub(super) backend: Rc<RefCell<Box<dyn PlayerBackend>>>,
    pub(super) app_state: Signal<EpisodeState>,
    /// Playlists + queue resolution (its own signal, split from `EpisodeState`).
    /// Peeked by the transport navigation (queue continuation / "up next") and
    /// auto-advance — the player walks the queue without subscribing to it.
    pub(super) playlists: Signal<PlaylistState>,
    /// Playback cursors overlay (its own signal, split from `EpisodeState`). Peeked at
    /// play time to resume from the saved cursor (`playback_for`).
    pub(super) playbacks: Signal<PlaybackOverlay>,
    /// Device/server download state (its own signal, split from `EpisodeState`).
    /// Peeked when resolving a play source and by the preparing-download watcher —
    /// the player needs the device-copy + progress, not the rest of `EpisodeState`.
    pub(super) downloads: Signal<DownloadState>,
    pub(super) now_playing: Signal<Option<NowPlaying>>,
    pub(super) sync_handle: Coroutine<Command>,
    tick_count: Rc<Cell<u32>>,
    /// Device byte storage — where local playback sources come from.
    pub(super) media: Option<Rc<dyn MediaStore>>,
    /// Client config (playback preference + server_url/access_token for the
    /// stream URL). Peeked at play time, never subscribed — a settings change
    /// applies to the *next* play.
    pub(super) config: Signal<ClientConfig>,
    /// Monotonic guard for async source resolution: each load bumps it, and a
    /// stale in-flight resolution (user already tapped another episode) checks
    /// it before touching the backend.
    pub(super) load_generation: Rc<Cell<u64>>,
    /// Sleep timer — ticked down with playback, pauses on expiry. Player-level
    /// (not per-episode), so it spans auto-advance/queue transitions.
    sleep: Signal<SleepState>,
    /// The playback continuation context (see [`PlayContext`]): set by the
    /// user-entry `*_in` play methods, preserved by the continuation paths
    /// (auto-advance, transport next/prev, stream fallback), cleared by
    /// [`Self::stop`]. Peeked by navigation/auto-advance; UI "up next" surfaces
    /// subscribe via their own hook.
    play_context: Signal<PlayContext>,
    /// Latches once `sleep_by_default` has auto-armed for the current listening
    /// session; reset by [`Self::stop`]. Keeps auto-arm to once-per-session so a
    /// manual disable isn't undone by the next episode.
    sleep_auto_armed: Rc<Cell<bool>>,
    /// Consecutive ticks spent in `Loading` (the first `TimeUpdate` never
    /// arrived). Guards against a remote stream that never reaches `HAVE_METADATA`
    /// wedging the player in `Loading` forever; reset whenever state leaves
    /// `Loading`.
    loading_ticks: Rc<Cell<u32>>,
    /// Consecutive ticks spent in `Preparing` (a device download is in flight but
    /// hasn't reached `Downloaded`/`Failed`). Guards against a dropped
    /// `DownloadToDevice` command or a worker that never reaches a terminal state
    /// wedging the spinner forever — `should_poll` skips `Preparing`, so the
    /// loading-stall guard never sees it. Reset whenever state leaves `Preparing`,
    /// or when the download reports progress (`note_preparing_progress`).
    preparing_ticks: Rc<Cell<u32>>,
    /// Whether the source currently in the backend is a device copy
    /// (`MediaSource::Local`) — set by `load_and_play`, read by the local-error
    /// retry (only device loads are retried; a stream error is surfaced as-is).
    last_source_local: Rc<Cell<bool>>,
    /// Local-source load failures retried so far for the current play intent
    /// (see `on_backend_error`). Reset by a fresh user play (`request_play`),
    /// a successful start (`Loading → Playing`), and `stop`.
    local_retries: Rc<Cell<u32>>,
    /// A local-error retry is scheduled but hasn't fired: `tick` discards
    /// backend events (the aborted load's stragglers — a late play-promise
    /// rejection would otherwise burn a second retry) and freezes the state
    /// machine in `Loading` until the retry re-resolves the source. Cleared by
    /// the retry firing or by any new load (`bump_generation`).
    retry_pending: Rc<Cell<bool>>,
}

/// Ticks (`TICK_INTERVAL_MS` apart) the player may sit in `Loading` before the
/// stall guard surfaces an error. ~30s at 250ms/tick.
const LOADING_STALL_TICKS: u32 = 30_000 / TICK_INTERVAL_MS as u32;

/// Ticks (`TICK_INTERVAL_MS` apart) the player may sit in `Preparing` without the
/// device download reaching a terminal state before the stall guard surfaces an
/// error. Generous (downloads legitimately take a while) but bounded, so a dropped
/// `DownloadToDevice` command or a stuck worker can't hang the spinner forever.
/// ~5min at 250ms/tick.
const PREPARING_STALL_TICKS: u32 = 300_000 / TICK_INTERVAL_MS as u32;

/// Backoff (ms) before each local-source error retry; the array length IS the
/// retry budget. Device-copy loads can fail transiently on iOS WebKit (a
/// freshly-committed IndexedDB blob whose backing file hasn't settled — see
/// `ui-svc-media`'s probe notes), and re-resolving the source a moment later
/// succeeds; a genuinely corrupt copy exhausts the budget and surfaces the
/// error as before. Short enough that the whole ladder stays inside the
/// Loading spinner without feeling stuck.
const LOCAL_RETRY_DELAYS_MS: [u32; 2] = [750, 2_000];

impl PlayerController {
    pub fn new(
        backend: Box<dyn PlayerBackend>,
        app_state: Signal<EpisodeState>,
        playlists: Signal<PlaylistState>,
        playbacks: Signal<PlaybackOverlay>,
        downloads: Signal<DownloadState>,
        now_playing: Signal<Option<NowPlaying>>,
        sync_handle: Coroutine<Command>,
        media: Option<Rc<dyn MediaStore>>,
        config: Signal<ClientConfig>,
        sleep: Signal<SleepState>,
        play_context: Signal<PlayContext>,
    ) -> Self {
        Self {
            backend: Rc::new(RefCell::new(backend)),
            app_state,
            playlists,
            playbacks,
            downloads,
            now_playing,
            sync_handle,
            tick_count: Rc::new(Cell::new(0)),
            media,
            config,
            load_generation: Rc::new(Cell::new(0)),
            sleep,
            play_context,
            sleep_auto_armed: Rc::new(Cell::new(false)),
            loading_ticks: Rc::new(Cell::new(0)),
            preparing_ticks: Rc::new(Cell::new(0)),
            last_source_local: Rc::new(Cell::new(false)),
            local_retries: Rc::new(Cell::new(0)),
            retry_pending: Rc::new(Cell::new(false)),
        }
    }

    /// Bump and return the load generation (called at the start of any load).
    pub(super) fn bump_generation(&self) -> u64 {
        // Any new load supersedes a pending local-error retry: un-freeze the
        // tick loop immediately; the sleeping retry task then sees the changed
        // generation and aborts.
        self.retry_pending.set(false);
        let g = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(g);
        g
    }

    /// The active playback preference (peeked — no subscription).
    ///
    /// Embedded-server mode forces `StreamOnly` at this single chokepoint: the
    /// "server" is this device, so a device copy would duplicate every byte
    /// for nothing — streaming from loopback IS local playback. The stored
    /// preference is left untouched so switching back to a remote account
    /// restores the user's real setting.
    pub(super) fn preference(&self) -> PlaybackPreference {
        let cfg = self.config.peek();
        if cfg.server_kind.is_embedded() {
            return PlaybackPreference::StreamOnly;
        }
        cfg.playback_prefs.playback_preference
    }

    /// The saved default playback rate (peeked — applied to each new load).
    fn rate(&self) -> f32 {
        self.config.peek().playback_prefs.playback_rate
    }

    /// Set the play context (value-gated write, so re-playing from the same
    /// list doesn't re-render the "up next" subscribers).
    pub(super) fn set_play_context(&self, ctx: Option<i32>) {
        let next = PlayContext(ctx);
        if *self.play_context.peek() != next {
            let mut play_context = self.play_context;
            play_context.set(next);
        }
    }

    /// The current play context playlist id (peeked — never subscribes).
    pub fn play_context(&self) -> Option<i32> {
        self.play_context.peek().0
    }

    pub(super) fn set_now_playing(&self, np: NowPlaying) {
        let mut now_playing = self.now_playing;
        now_playing.set(Some(np));
    }

    fn update_now_playing(&self, f: impl FnOnce(&mut NowPlaying)) {
        // `peek()` avoids subscribing/notifying. `Signal::write()`/`set()` notify
        // every subscriber on drop even when nothing changed, and the web backend
        // re-reports DurationKnown/Buffering each poll (~4×/sec); value-gate the
        // write so an unchanged poll (or a paused player) doesn't re-render the
        // player UI. `now_playing` has dedicated player-UI subscribers (not the
        // shared EpisodeState), so gating it is safe.
        let Some(mut next) = self.now_playing.peek().clone() else {
            return;
        };
        f(&mut next);
        if self.now_playing.peek().as_ref() != Some(&next) {
            // Rebind to a local `mut` (the signal is `Copy`) so `set` has a mutable
            // receiver — `self` is `&self`, so the field can't be borrowed mutably.
            let mut now_playing = self.now_playing;
            now_playing.set(Some(next));
        }
    }

    /// The saved cursor for an episode (peeked — playback start, not reactive).
    /// Overlay-wins via `playback_for`: a locally-written cursor beats the one the
    /// server embedded on the cached body. No bulk playback pull required — the
    /// episode being played is in the pool (rendered, or fetched by episode-detail).
    pub(super) fn saved_cursor(&self, episode_id: i32) -> f64 {
        self.playbacks
            .peek()
            .playback_for(&self.app_state.peek(), episode_id)
            .map(|p| p.cursor.max(0) as f64)
            .unwrap_or(0.0)
    }

    /// Load `src` into the backend and reflect `Loading`.
    pub(super) fn load_and_play(&self, episode_id: i32, src: MediaSource, start_at: f64) {
        // Every real playback start funnels through here (device + stream, whether
        // from the transport, the queue auto-advance, the "Stream" menu/swipe, or the
        // download-then-play watcher), so this is the one place to auto-arm the sleep
        // timer. Idempotent via the once-per-session latch, so auto-advance/replays
        // don't re-arm it.
        self.maybe_auto_arm_sleep();
        let local = matches!(&src, MediaSource::Local(_));
        // Remembered for the local-error retry: only device-copy loads are
        // retried on a backend error (see `on_backend_error`).
        self.last_source_local.set(local);
        let kind = if local { "device" } else { "stream" };
        debug!(episode_id, source = kind, start_at, "Loading audio source");
        let rate = self.rate();
        {
            let b = self.backend.borrow();
            b.load(src, start_at);
            // `load()` resets the element's playbackRate to its default (1.0x), so
            // re-apply the saved rate before playing — otherwise the user's chosen
            // speed is silently lost on every track change.
            b.set_rate(rate);
            b.play();
        }
        self.set_now_playing(NowPlaying::loading(episode_id, start_at, rate));
    }

    pub(super) fn set_error(&self, episode_id: i32, msg: &str) {
        warn!(episode_id, error = msg, "Playback error");
        self.set_now_playing(NowPlaying::error(episode_id, msg));
    }

    /// Start a fresh local-error retry budget — called on each fresh user play
    /// intent (`request_play`), so one episode's exhausted budget can't bleed
    /// into the next tap.
    pub(super) fn reset_local_retries(&self) {
        self.local_retries.set(0);
    }

    /// Route a backend `PlayerEvent::Error`: device-copy loads get a bounded
    /// retry (fresh source resolution — a fresh IndexedDB read and a fresh
    /// object URL) before the error is surfaced, because a freshly-committed
    /// blob can transiently fail to load on iOS WebKit while a re-read moments
    /// later succeeds (see `LOCAL_RETRY_DELAYS_MS`). Stream errors and
    /// exhausted budgets latch `Error` exactly as before.
    fn on_backend_error(&self, msg: String) {
        let snapshot = self
            .now_playing
            .peek()
            .as_ref()
            .map(|n| (n.episode_id, n.state.clone()));
        let attempt = self.local_retries.get() as usize;
        // Only retry a device load that failed to *start* (Loading): a mid-play
        // error on an already-playing element is a different beast, and a
        // pending retry means this event is a straggler of the load we already
        // gave up on.
        if let Some((episode_id, PlaybackState::Loading)) = snapshot
            && self.last_source_local.get()
            && attempt < LOCAL_RETRY_DELAYS_MS.len()
            && !self.retry_pending.get()
        {
            let delay_ms = LOCAL_RETRY_DELAYS_MS[attempt];
            self.local_retries.set(attempt as u32 + 1);
            self.retry_pending.set(true);
            warn!(
                episode_id,
                error = %msg,
                attempt = attempt + 1,
                delay_ms,
                "Device playback failed; retrying with a fresh source"
            );
            let generation = self.load_generation.get();
            let this = self.clone();
            spawn(async move {
                halogen_ui_platform::time::sleep_ms(delay_ms).await;
                // Superseded (new load bumped the generation / cleared the
                // flag) or the user changed state (pause/stop) meanwhile —
                // don't yank playback around.
                if !this.retry_pending.get() || this.load_generation.get() != generation {
                    return;
                }
                this.retry_pending.set(false);
                let still_loading = this.now_playing.peek().as_ref().is_some_and(|n| {
                    n.episode_id == episode_id && n.state == PlaybackState::Loading
                });
                if still_loading {
                    this.play_episode(episode_id);
                }
            });
            return;
        }
        warn!(error = %msg, "Backend playback error");
        self.update_now_playing(|n| n.state = PlaybackState::Error(msg));
    }

    /// The current coarse playback state, or `None` when nothing is loaded.
    fn playback_state(&self) -> Option<PlaybackState> {
        self.now_playing.read().as_ref().map(|n| n.state.clone())
    }

    /// The current episode id + coarse state, or `None` when nothing is loaded.
    /// `read()` (not `peek()`) so a transport rendered from this state stays
    /// subscribed, matching what `toggle` has always done.
    fn now_playing_snapshot(&self) -> Option<(i32, PlaybackState)> {
        self.now_playing
            .read()
            .as_ref()
            .map(|n| (n.episode_id, n.state.clone()))
    }

    /// Play/pause toggle — the in-app transport (mini-player, now-playing sheet,
    /// episode rows).
    ///
    /// Both directions delegate to [`Self::pause`] / [`Self::play`], so the
    /// "what does play mean from state X" routing lives in exactly ONE place and
    /// can't drift from the OS media-session handlers, which drive those two
    /// directional intents separately rather than as a toggle.
    pub fn toggle(&self) {
        let Some((_, state)) = self.now_playing_snapshot() else {
            return;
        };
        match state {
            // Playing → stop. `Loading` too: autoplay is already in flight and a
            // tap cancels it.
            PlaybackState::Playing | PlaybackState::Loading => self.pause(),
            // Paused → resume; `Ended`/`Error` → replay; `Preparing` → left alone.
            _ => self.play(),
        }
    }

    /// The **directional** "start playing" intent: the OS media-session `play`
    /// action, plus any control that means *play* rather than *toggle*. The
    /// counterpart of [`Self::pause`], which is already directional (it acts from
    /// `Playing`/`Loading` and ignores everything else).
    ///
    /// State-aware, because [`Self::resume`] alone is NOT enough: it is a
    /// deliberate no-op outside `Paused`, so wiring the OS play button straight to
    /// it left that button dead after a track finished (`Ended`) or a play failed
    /// (`Error`) — while the OS pause button kept working, since `pause` accepts
    /// `Playing`. That asymmetry is invisible to the in-app transport, which only
    /// ever reaches these states through `toggle`.
    pub fn play(&self) {
        let Some((episode_id, state)) = self.now_playing_snapshot() else {
            return;
        };
        match state {
            PlaybackState::Paused => self.resume(),
            // Finished or errored: "play" means start it over / retry. `Error` is
            // otherwise absorbing — `should_poll` stops polling it and `resume`
            // refuses it — so `request_play` is the only way back out.
            PlaybackState::Ended | PlaybackState::Error(_) => self.request_play(episode_id),
            // Already playing, or already headed there.
            PlaybackState::Playing | PlaybackState::Loading => {}
            // Download-before-play: the `PlayerProvider` watcher starts playback
            // once the bytes land. Flipping state here wedges that handoff (the
            // watcher requires `Preparing`) and lets the next cursor persist zero
            // the saved resume position.
            PlaybackState::Preparing => {}
        }
    }

    /// Resume a **paused** player — the low-level primitive behind [`Self::play`].
    /// Guarded to `Paused` only: from any other state a flip to `Playing` is wrong
    /// (`Preparing` wedges the download handoff and zeroes the saved cursor;
    /// `Loading` is already headed to `Playing`; `Ended`/`Error` need a real
    /// re-play). Callers that mean "start playing" want [`Self::play`], which
    /// routes those states correctly instead of silently doing nothing.
    pub fn resume(&self) {
        if self.playback_state() != Some(PlaybackState::Paused) {
            return;
        }
        debug!("Resume");
        self.backend.borrow().play();
        self.update_now_playing(|n| n.state = PlaybackState::Playing);
    }

    /// Pause a **playing/loading** player. Guarded for the same reason as
    /// [`Self::resume`]: from `Preparing` a flip to `Paused` bypasses
    /// `persist_cursor`'s `Preparing|Error` skip and writes cursor 0, erasing the
    /// episode's saved resume position; `Paused`/`Ended`/`Error` have nothing to
    /// pause.
    pub fn pause(&self) {
        match self.playback_state() {
            Some(PlaybackState::Playing | PlaybackState::Loading) => {}
            _ => return,
        }
        debug!("Pause");
        self.backend.borrow().pause();
        self.update_now_playing(|n| n.state = PlaybackState::Paused);
        self.persist_cursor();
    }

    // ── Sleep timer ─────────────────────────────────────────────────────────
    //
    // The timer's state + rules live in `sleep.rs`; these methods keep the public
    // surface stable and forward to the `sleep::*` orchestration over the
    // `Signal<SleepState>` (the controller still owns the signal + the
    // once-per-session auto-arm latch, and pauses on the expiry edge `advance`
    // reports).

    /// The sleep-timer state signal. The UI reads it (active flag + remaining
    /// minutes for the badge); all mutation goes through the methods below.
    pub fn sleep_state(&self) -> Signal<SleepState> {
        self.sleep
    }

    /// The configured default sleep duration (minutes), peeked.
    fn default_sleep_minutes(&self) -> i64 {
        self.config.peek().playback_prefs.default_sleep_minutes as i64
    }

    /// Disable the sleep timer.
    pub fn disable_sleep(&self) {
        sleep::disable(self.sleep);
    }

    /// Toggle the timer: active → off; off → armed with the configured default.
    pub fn toggle_sleep(&self) {
        sleep::toggle(self.sleep, self.default_sleep_minutes());
    }

    /// Nudge the remaining time by `delta_minutes` (clamped at 0 → off; from off, a
    /// positive delta starts a fresh timer).
    pub fn adjust_sleep(&self, delta_minutes: i64) {
        sleep::adjust(self.sleep, delta_minutes);
    }

    /// Auto-arm the timer once per session when `sleep_by_default` is set and nothing
    /// is armed yet. Called when playback is requested; the latch keeps it from
    /// re-arming on each episode, so the timer spans the whole session.
    fn maybe_auto_arm_sleep(&self) {
        sleep::maybe_auto_arm(
            self.sleep,
            &self.sleep_auto_armed,
            self.config.peek().playback_prefs.sleep_by_default,
            self.default_sleep_minutes(),
        );
    }

    /// Advance the timer by `dt` seconds, pausing once on expiry. Called from `tick`
    /// only while actually playing. Returns `true` on the tick the timer expired (so
    /// the caller can suppress a coincident auto-advance).
    fn advance_sleep(&self, dt: f64) -> bool {
        if sleep::advance(self.sleep, dt) {
            self.pause();
            true
        } else {
            false
        }
    }

    pub fn seek_to(&self, secs: f64) {
        debug!(secs, "Seek");
        self.backend.borrow().seek(secs);
        self.update_now_playing(|n| n.position_secs = secs);
    }

    /// Seek by a relative offset (pairs with the absolute [`Self::seek_to`]).
    pub fn seek_relative(&self, delta_secs: f64) {
        let pos = self
            .now_playing
            .read()
            .as_ref()
            .map(|n| n.position_secs)
            .unwrap_or(0.0);
        self.seek_to((pos + delta_secs).max(0.0));
    }

    /// The episode `delta` (+1/-1) places away from the current one in the
    /// active list (delegates to [`navigation::adjacent_episode`]). `None` at the
    /// ends of the list or when nothing is playing.
    fn adjacent_episode(&self, delta: i32) -> Option<i32> {
        let current = self.current_episode()?;
        navigation::adjacent_episode(
            &self.app_state.peek(),
            &self.playlists.peek(),
            current,
            delta,
            self.play_context(),
        )
    }

    /// The currently-playing episode id, if any. `peek` — never subscribes.
    fn current_episode(&self) -> Option<i32> {
        self.now_playing.peek().as_ref().map(|n| n.episode_id)
    }

    /// The transport Next target (delegates to [`navigation::next_episode`]): the
    /// playlist/queue continuation ([`PlaylistState::next_up_in`] with the play
    /// context) when there is one, else the podcast-order neighbor so you can
    /// keep bingeing a podcast whose episode isn't queued. Auto-advance (on
    /// episode end) uses `next_up_in` directly — no podcast fallback — so it
    /// stops at the playlist's end.
    fn next_episode(&self) -> Option<i32> {
        navigation::next_episode(
            &self.app_state.peek(),
            &self.playlists.peek(),
            self.current_episode(),
            self.play_context(),
        )
    }

    /// Whether the episode `delta` steps away (in podcast order) exists.
    pub fn has_adjacent(&self, delta: i32) -> bool {
        self.adjacent_episode(delta).is_some()
    }

    /// Whether the transport Prev control has a target. Symmetric with
    /// [`Self::has_next`]; mirrors [`Self::play_previous_episode`].
    pub fn has_previous(&self) -> bool {
        self.has_adjacent(-1)
    }

    /// Whether the transport Next control has a target (queue continuation or a
    /// podcast neighbor). Mirrors [`Self::play_next_episode`].
    pub fn has_next(&self) -> bool {
        self.next_episode().is_some()
    }

    /// UI transport control: play the next episode (queue continuation first, then
    /// podcast order). Deliberately independent of the Bluetooth next/previous-track
    /// override — that only remaps hardware buttons (see media_session.rs).
    pub fn play_next_episode(&self) {
        match self.next_episode() {
            Some(id) => {
                debug!(episode_id = id, "Next track");
                self.request_play(id);
            }
            None => debug!("Next track: no next episode"),
        }
    }

    /// UI transport control: play the previous episode. See [`Self::play_next_episode`].
    pub fn play_previous_episode(&self) {
        match self.adjacent_episode(-1) {
            Some(id) => {
                debug!(episode_id = id, "Previous track");
                self.request_play(id);
            }
            None => debug!("Previous track: no previous episode"),
        }
    }

    pub fn set_rate(&self, rate: f32) {
        debug!(rate, "Set playback rate");
        self.backend.borrow().set_rate(rate);
        self.update_now_playing(|n| n.rate = rate);
    }

    pub fn stop(&self) {
        debug!("Stop");
        // Persist the cursor before tearing down so closing the player remembers
        // where we were (the app resumes from the saved cursor). Without this,
        // a play→close with no pause and no end-of-clip leaves no playback record
        // at all — the episode never reaches History and resume starts from 0.
        self.persist_cursor();
        self.backend.borrow().stop();
        let mut now_playing = self.now_playing;
        now_playing.set(None);
        // Closing the player ends the listening session: clear the play context
        // and sleep timer, and re-arm the once-per-session auto-arm latch.
        // Cancel any pending local-error retry with it — nothing is loaded to
        // retry anymore.
        self.retry_pending.set(false);
        self.reset_local_retries();
        self.set_play_context(None);
        self.disable_sleep();
        self.sleep_auto_armed.set(false);
    }

    pub fn persist_cursor(&self) {
        if let Some(np) = self.now_playing.peek().clone() {
            // `Preparing`/`Error` force `position_secs` to 0.0 and haven't played a
            // frame — persisting would clobber the episode's real saved resume
            // cursor with 0. `Ended` already had its terminal cursor written by
            // `on_ended`'s `MarkPlayed` (reset to 0 + Finished); persisting the
            // ~duration end position here (on `stop()` / tab-close from the Ended
            // screen) would UNDO that, so a replay resumes at the very end and
            // instantly re-finishes. Only persist states that reflect a live,
            // resumable position.
            if matches!(
                np.state,
                PlaybackState::Preparing | PlaybackState::Error(_) | PlaybackState::Ended
            ) {
                return;
            }
            self.sync_handle.send(Command::SetCursor {
                episode_id: np.episode_id,
                cursor: np.position_secs as i64,
            });
        }
    }

    /// Poll backend events into the `now_playing` signal. Called periodically by
    /// `PlayerProvider`. A short orchestrator over the per-tick helpers below.
    pub fn tick(&self) {
        // `Preparing` has no backend source loaded, so `should_poll` skips it — run
        // its stall guard here (before that skip) so a download that never reaches a
        // terminal state can't hang the spinner forever.
        if self.check_preparing_stall() {
            return;
        }
        // A local-error retry is pending: swallow the aborted load's straggler
        // events (a late play-promise rejection or the element's re-reported
        // error would burn a second retry — and its TimeUpdate would fake a
        // Loading→Playing flip) and hold the state machine in `Loading` until
        // the retry re-resolves the source. Bounded by LOCAL_RETRY_DELAYS_MS.
        if self.retry_pending.get() {
            let _ = self.backend.borrow_mut().poll_events();
            return;
        }
        if !self.should_poll() {
            return;
        }
        let ended = self.drain_events();
        if self.check_loading_stall() {
            return;
        }
        let sleep_expired = self.persist_while_playing();
        if ended {
            // Suppress auto-advance when the sleep timer expired on this same tick:
            // the user asked to stop, so respect that over the queue continuation
            // (the track is still marked played + latched to `Ended`).
            self.on_ended(!sleep_expired);
        }
    }

    /// Whether this tick should poll the backend. `false` (skip) for the
    /// terminal/idle states. Without this, the backend's per-tick
    /// TimeUpdate/Buffering events would write the signal 4×/sec and re-render the
    /// player while idle; and once a track ends the element keeps reporting
    /// `ended` every poll, so without latching on the terminal `Ended` state we'd
    /// re-fire `MarkPlayed` (and re-publish EpisodeState) forever. `peek()` doesn't
    /// subscribe or notify.
    fn should_poll(&self) -> bool {
        match self.now_playing.peek().as_ref().map(|n| &n.state) {
            None => false,
            Some(PlaybackState::Ended) => false,
            // Error is terminal too: the element keeps reporting the same error
            // every poll, so without latching we'd re-warn + re-emit forever. A
            // retry funnels through `request_play`, which resets state to Loading.
            Some(PlaybackState::Error(_)) => false,
            // Preparing has no backend source loaded yet — polling would read the
            // previous episode's element and bleed stale events into this state.
            Some(PlaybackState::Preparing) => false,
            _ => true,
        }
    }

    /// Drain the backend's polled events into `now_playing`. Returns whether the
    /// track reported `Ended` this tick (latched for the caller).
    fn drain_events(&self) -> bool {
        let events = self.backend.borrow_mut().poll_events();
        let mut ended = false;
        for ev in events {
            match ev {
                PlayerEvent::TimeUpdate(secs) => {
                    let was_loading = matches!(
                        self.now_playing.peek().as_ref().map(|n| &n.state),
                        Some(PlaybackState::Loading)
                    );
                    self.update_now_playing(|n| {
                        n.position_secs = secs;
                        n.buffering = false;
                        // First clock tick after load means playback actually started.
                        if n.state == PlaybackState::Loading {
                            n.state = PlaybackState::Playing;
                        }
                    });
                    // Playback genuinely started — the local-error retry budget
                    // is for *this* start, so a later transient failure (next
                    // episode, replay) gets a fresh one.
                    if was_loading {
                        self.reset_local_retries();
                    }
                }
                PlayerEvent::DurationKnown(d) => {
                    self.update_now_playing(|n| n.duration_secs = Some(d))
                }
                PlayerEvent::Buffering(b) => self.update_now_playing(|n| n.buffering = b),
                // The OS paused the element on its own (audio-route change when a
                // speaker/headset disconnects, an incoming call, another app
                // taking the audio focus). Only act on the Playing→Paused edge:
                // our own `pause()` already set Paused (so this is a no-op then),
                // and a still-Loading element hasn't started playing yet. Persist
                // the cursor as a real pause would, so the position isn't lost.
                PlayerEvent::Paused => {
                    if matches!(
                        self.now_playing.peek().as_ref().map(|n| &n.state),
                        Some(PlaybackState::Playing)
                    ) {
                        self.update_now_playing(|n| n.state = PlaybackState::Paused);
                        self.persist_cursor();
                    }
                }
                // The OS resumed the element on its own (handed audio focus back).
                // Mirror of `Paused`: only act on the Paused→Playing edge so our
                // own `resume()` (already Playing) and a Loading element are left
                // alone — Loading→Playing is owned by the first `TimeUpdate`.
                PlayerEvent::Playing => {
                    if matches!(
                        self.now_playing.peek().as_ref().map(|n| &n.state),
                        Some(PlaybackState::Paused)
                    ) {
                        self.update_now_playing(|n| n.state = PlaybackState::Playing);
                    }
                }
                PlayerEvent::Ended => ended = true,
                // Routed through the local-error retry: a device-copy load gets
                // a bounded re-resolve before the error latches (see
                // `on_backend_error`); stream errors surface immediately.
                PlayerEvent::Error(e) => self.on_backend_error(e),
            }
        }
        ended
    }

    /// Loading-stall guard: the controller only flips `Loading → Playing` on the
    /// first `TimeUpdate`. If a remote stream never reaches `HAVE_METADATA`
    /// (stalled network), that tick never comes and the player spins in `Loading`
    /// forever — surface an error after ~30s instead. Returns `true` when the
    /// stall fired (the rest of the tick should be skipped).
    fn check_loading_stall(&self) -> bool {
        if matches!(
            self.now_playing.peek().as_ref().map(|n| &n.state),
            Some(PlaybackState::Loading)
        ) {
            let n = self.loading_ticks.get().wrapping_add(1);
            self.loading_ticks.set(n);
            if n >= LOADING_STALL_TICKS {
                self.loading_ticks.set(0);
                let id = self.current_episode().unwrap_or_default();
                self.backend.borrow().stop();
                self.set_error(id, "Playback timed out — couldn't start this episode.");
                return true;
            }
        } else {
            self.loading_ticks.set(0);
        }
        false
    }

    /// Preparing-stall guard: mirror of [`Self::check_loading_stall`] for the
    /// `Preparing` state. The `Preparing → play / Error` transition is event-driven
    /// (the `PlayerProvider` watcher), so a dropped `DownloadToDevice` command or a
    /// worker that never reaches `Downloaded`/`Failed` would spin the spinner
    /// forever — surface an error after `PREPARING_STALL_TICKS` instead. Returns
    /// `true` while in `Preparing` (no backend to poll this tick) so `tick` stops.
    fn check_preparing_stall(&self) -> bool {
        if matches!(
            self.now_playing.peek().as_ref().map(|n| &n.state),
            Some(PlaybackState::Preparing)
        ) {
            let n = self.preparing_ticks.get().wrapping_add(1);
            self.preparing_ticks.set(n);
            if n >= PREPARING_STALL_TICKS {
                self.preparing_ticks.set(0);
                let id = self.current_episode().unwrap_or_default();
                self.backend.borrow().stop();
                self.set_error(id, "Download timed out — couldn't prepare this episode.");
            }
            return true;
        }
        self.preparing_ticks.set(0);
        false
    }

    /// Reset the `Preparing` stall window — called when the watched device download
    /// reports forward progress (`Downloading`), so a slow-but-advancing download
    /// gets a fresh window rather than timing out from when `Preparing` was entered.
    pub(super) fn note_preparing_progress(&self) {
        self.preparing_ticks.set(0);
    }

    /// Debounced cursor persistence (~every 40th tick ≈ 10s), and only while
    /// actually playing — a paused player isn't advancing, so there's nothing new
    /// to save. Each persist is a `SetCursor` command that publishes `EpisodeState`,
    /// so keep it coarse to avoid re-rendering the episode lists.
    /// Returns `true` when the sleep timer expired on this tick (so `tick` can skip a
    /// coincident auto-advance).
    fn persist_while_playing(&self) -> bool {
        let is_playing = matches!(
            self.now_playing.peek().as_ref().map(|n| &n.state),
            Some(PlaybackState::Playing)
        );
        if !is_playing {
            return false;
        }
        // Count the sleep timer down in step with the tick cadence (only while
        // playing — a paused player holds the timer). Pauses on expiry.
        let sleep_expired = self.advance_sleep(TICK_INTERVAL_MS as f64 / 1000.0);

        let c = self.tick_count.get().wrapping_add(1);
        self.tick_count.set(c);
        if c.is_multiple_of(40) {
            self.persist_cursor();
        }
        sleep_expired
    }

    /// Settle the track that just reported `Ended`: mark it played, latch to the
    /// terminal `Ended` state (so `should_poll` stops and the per-poll `ended`
    /// re-fire can't re-trigger this), then auto-advance to the next item in the
    /// continuation playlist (the play context, else the queue) when the setting
    /// is on and one exists (`next_up_in` only — no podcast fallback, so it
    /// stops at the playlist's end). With no next, settle on the `Ended` state
    /// set here.
    fn on_ended(&self, allow_advance: bool) {
        let current = self.current_episode();
        if let Some(id) = current {
            info!(episode_id = id, "Episode ended, marking played");
            self.sync_handle.send(Command::MarkPlayed {
                episode_id: id,
                played: true,
            });
        }
        // Latch out of the pollable Playing state *synchronously*, before routing
        // the next play. The async `play_episode` path (device copy / lost-bytes
        // fallback) doesn't touch `now_playing` until its spawned source resolves,
        // so without this the still-Playing <audio> re-reports `ended` on the next
        // poll and re-fires MarkPlayed + request_play until the spawn lands. `Ended`
        // makes `should_poll` false, closing that re-detect window; `request_play`
        // overwrites it (Loading/Preparing) once it resolves. The synchronous
        // StreamOnly/`enter_preparing` paths already set state immediately and so
        // never had this window.
        self.update_now_playing(|n| n.state = PlaybackState::Ended);

        // `allow_advance` is false when the sleep timer expired on this same tick —
        // the user's "stop after this" intent wins over the queue continuation.
        let auto_advance = allow_advance && self.config.peek().playback_prefs.auto_advance;
        let next = auto_advance
            .then(|| {
                self.playlists
                    .peek()
                    .next_up_in(current, self.play_context())
            })
            .flatten();
        // No next in the continuation playlist: stay on the terminal `Ended`
        // state set above.
        if let Some(id) = next {
            info!(episode_id = id, "Auto-advancing to next in playlist");
            self.request_play(id);
        }
    }
}
