//! In-process reactivity tests for [`PlayerController`].
//!
//! These drive a real controller inside a `VirtualDom` runtime (so `spawn`,
//! signals, and the coroutine channel all behave as in the app) using a
//! recording [`PlayerBackend`] + recording [`MediaStore`] and a coroutine that
//! drains `Command`s into a shared `Vec`. The harness mirrors
//! `tests/render_semantics.rs`: a root component stashes the pieces the test
//! needs into `thread_local`s (nextest runs one process per test, so they can't
//! bleed), the test writes signals via `vdom.in_runtime(..)`, and `pump` runs
//! the event loop until it goes quiet.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dioxus::prelude::*;
use halogen_wire::{DownloadStatus, EpisodeData, PlaybackStatus, PlaylistData};

use crate::{NowPlaying, PlayContext, PlaybackState, PlayerController, ScopeBound, SleepState};
// Aliased: `PlaybackState` (above) is the player state machine; the worker-owned
// cursor overlay slice imports under a distinct name.
use halogen_ui_appstate::PlaybackState as PlaybackOverlay;
use halogen_ui_appstate::{ClientDownloadState, DownloadState, EpisodeState, PlaylistState};
use halogen_ui_commands::Command;
use halogen_ui_config::{ClientConfig, PlaybackPreference, ServerKind};
use halogen_ui_svc_media::{LocalAudio, MediaStore, MediaWriter, PartialInfo};

use super::{MediaSource, PlayerBackend, PlayerEvent};

// ── Recording backend ───────────────────────────────────────────────────────

/// A recorded backend method call.
#[derive(Debug, Clone, PartialEq)]
enum BackendCall {
    Load { remote: bool, start_at: f64 },
    Play,
    Pause,
    Seek(f64),
    SetRate(f32),
    Stop,
    Prime,
}

#[derive(Clone, Default)]
struct Recorder {
    calls: Rc<RefCell<Vec<BackendCall>>>,
    /// Scripted event batches; each `poll_events()` pops the front batch.
    events: Rc<RefCell<VecDeque<Vec<PlayerEvent>>>>,
}

impl Recorder {
    fn queue_events(&self, batch: Vec<PlayerEvent>) {
        self.events.borrow_mut().push_back(batch);
    }
    fn calls(&self) -> Vec<BackendCall> {
        self.calls.borrow().clone()
    }
}

struct RecordingBackend(Recorder);

impl PlayerBackend for RecordingBackend {
    fn load(&self, src: MediaSource, start_at_secs: f64) {
        let remote = matches!(src, MediaSource::Remote(_));
        self.0.calls.borrow_mut().push(BackendCall::Load {
            remote,
            start_at: start_at_secs,
        });
    }
    fn play(&self) {
        self.0.calls.borrow_mut().push(BackendCall::Play);
    }
    fn pause(&self) {
        self.0.calls.borrow_mut().push(BackendCall::Pause);
    }
    fn seek(&self, secs: f64) {
        self.0.calls.borrow_mut().push(BackendCall::Seek(secs));
    }
    fn set_rate(&self, rate: f32) {
        self.0.calls.borrow_mut().push(BackendCall::SetRate(rate));
    }
    fn stop(&self) {
        self.0.calls.borrow_mut().push(BackendCall::Stop);
    }
    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        self.0.events.borrow_mut().pop_front().unwrap_or_default()
    }
    fn prime(&self) {
        self.0.calls.borrow_mut().push(BackendCall::Prime);
    }
}

// ── Recording media store ───────────────────────────────────────────────────

/// A media store that returns a fixed local URL for any id with stored bytes.
#[derive(Default)]
struct RecordingMediaStore {
    /// Ids whose bytes are "present" → `audio_url` returns `Some`.
    present: RefCell<Vec<i32>>,
}

#[async_trait(?Send)]
impl MediaStore for RecordingMediaStore {
    async fn open_writer(
        &self,
        _episode_id: i32,
        _content_type: Option<&str>,
        _total: Option<u64>,
        _resume: bool,
    ) -> anyhow::Result<Box<dyn MediaWriter>> {
        Ok(Box::new(NoopMediaWriter))
    }
    async fn audio_url(&self, episode_id: i32) -> anyhow::Result<Option<LocalAudio>> {
        Ok(self
            .present
            .borrow()
            .contains(&episode_id)
            .then(|| LocalAudio {
                url: format!("blob:device/{episode_id}"),
                content_type: Some("audio/mpeg".to_string()),
            }))
    }
    async fn remove_audio(&self, _episode_id: i32) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list_ids(&self) -> anyhow::Result<Vec<i32>> {
        Ok(self.present.borrow().clone())
    }
    async fn partial(&self, _episode_id: i32) -> anyhow::Result<Option<PartialInfo>> {
        Ok(None)
    }
    async fn list_partials(&self) -> anyhow::Result<Vec<i32>> {
        Ok(Vec::new())
    }
    async fn clear(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// No-op writer for the recording store — these tests exercise playback, not the
/// download path, so accepted bytes are simply discarded.
struct NoopMediaWriter;

#[async_trait(?Send)]
impl MediaWriter for NoopMediaWriter {
    async fn write(&mut self, _chunk: &[u8]) -> anyhow::Result<()> {
        Ok(())
    }
    async fn commit(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

// ── Shared harness state (stashed by the root component) ─────────────────────

/// A controller call routed through the harness's effect so it runs inside a
/// component scope — `PlayerController::play_episode`/`stream_episode` call
/// dioxus `spawn`, which needs a *current scope* (not just the runtime), so
/// these can't be invoked from a bare `vdom.in_runtime`.
#[derive(Clone, Copy)]
enum Action {
    RequestPlay(i32),
    /// `request_play_in` — a user-entry play carrying a play context (the
    /// playlist the tap came from; `None` resets to queue semantics).
    RequestPlayIn(i32, Option<i32>),
    DownloadAndPlay(i32),
    NextTrack,
    PreviousTrack,
    /// The directional play intent (the OS media-session `play` action). Routed
    /// through here because it can land on `request_play` (Ended/Error → replay),
    /// which spawns.
    Play,
    Toggle,
    /// One controller poll tick. Routed through here (unlike the older
    /// bare-`in_runtime` tick tests) for paths where `tick` itself spawns —
    /// the local-error retry schedules its delayed re-resolve from
    /// `drain_events`, which needs a current scope, exactly like production's
    /// `use_future` tick loop provides.
    Tick,
}

thread_local! {
    static CONTROLLER: RefCell<Option<PlayerController>> = const { RefCell::new(None) };
    static APP_STATE: RefCell<Option<Signal<EpisodeState>>> = const { RefCell::new(None) };
    static PLAYLISTS: RefCell<Option<Signal<PlaylistState>>> = const { RefCell::new(None) };
    static DOWNLOADS: RefCell<Option<Signal<DownloadState>>> = const { RefCell::new(None) };
    static NOW_PLAYING: RefCell<Option<Signal<Option<NowPlaying>>>> = const { RefCell::new(None) };
    static COMMANDS: RefCell<Option<Rc<RefCell<Vec<Command>>>>> = const { RefCell::new(None) };
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
    static MEDIA: RefCell<Option<Rc<RecordingMediaStore>>> = const { RefCell::new(None) };
    static CONFIG: RefCell<Option<Signal<ClientConfig>>> = const { RefCell::new(None) };
    /// Runtime+scope captured inside the Harness scope — what production
    /// `media_session::register` captures for its browser-invoked handlers.
    static SCOPE_BOUND: RefCell<Option<ScopeBound>> = const { RefCell::new(None) };
    /// Queue of scope-requiring actions drained by the harness effect.
    static ACTIONS: RefCell<Vec<Action>> = const { RefCell::new(Vec::new()) };
    /// Trigger signal: bumping it re-runs the harness effect which drains ACTIONS.
    static TRIGGER: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
    /// Render counter for the now_playing probe (item: no-op write guard).
    static NP_RENDERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn reset_harness() {
    CONTROLLER.with(|c| *c.borrow_mut() = None);
    APP_STATE.with(|c| *c.borrow_mut() = None);
    PLAYLISTS.with(|c| *c.borrow_mut() = None);
    DOWNLOADS.with(|c| *c.borrow_mut() = None);
    NOW_PLAYING.with(|c| *c.borrow_mut() = None);
    COMMANDS.with(|c| *c.borrow_mut() = None);
    RECORDER.with(|c| *c.borrow_mut() = None);
    MEDIA.with(|c| *c.borrow_mut() = None);
    CONFIG.with(|c| *c.borrow_mut() = None);
    SCOPE_BOUND.with(|c| *c.borrow_mut() = None);
    ACTIONS.with(|c| c.borrow_mut().clear());
    TRIGGER.with(|c| *c.borrow_mut() = None);
    NP_RENDERS.with(|c| c.set(0));
}

/// Queue a scope-requiring action and pump it through the harness effect (which
/// runs inside a component scope so the controller's internal `spawn` works).
async fn run_action(vdom: &mut VirtualDom, action: Action, budget: u32) {
    ACTIONS.with(|c| c.borrow_mut().push(action));
    let mut trigger = TRIGGER.with(|c| c.borrow().expect("trigger stashed"));
    let n = vdom.in_runtime(|| *trigger.peek());
    vdom.in_runtime(move || trigger.set(n + 1));
    pump(vdom, budget).await;
}

fn controller() -> PlayerController {
    CONTROLLER.with(|c| c.borrow().clone().expect("controller stashed"))
}
fn app_state_sig() -> Signal<EpisodeState> {
    APP_STATE.with(|c| c.borrow().expect("app_state stashed"))
}
fn playlists_sig() -> Signal<PlaylistState> {
    PLAYLISTS.with(|c| c.borrow().expect("playlists stashed"))
}
fn downloads_sig() -> Signal<DownloadState> {
    DOWNLOADS.with(|c| c.borrow().expect("downloads stashed"))
}
fn now_playing_sig() -> Signal<Option<NowPlaying>> {
    NOW_PLAYING.with(|c| c.borrow().expect("now_playing stashed"))
}
fn commands() -> Vec<Command> {
    COMMANDS.with(|c| {
        c.borrow()
            .as_ref()
            .expect("commands stashed")
            .borrow()
            .clone()
    })
}
fn recorder() -> Recorder {
    RECORDER.with(|c| c.borrow().clone().expect("recorder stashed"))
}
fn media() -> Rc<RecordingMediaStore> {
    MEDIA.with(|c| c.borrow().clone().expect("media stashed"))
}
fn config_sig() -> Signal<ClientConfig> {
    CONFIG.with(|c| c.borrow().expect("config stashed"))
}
fn scope_bound() -> ScopeBound {
    SCOPE_BOUND.with(|c| c.borrow().clone().expect("scope_bound stashed"))
}

/// Pump the vdom until quiet (no work for 50ms) or `budget` passes. Lets the
/// drain coroutine + any `spawn`ed source resolution run.
async fn pump(vdom: &mut VirtualDom, budget: u32) {
    let mut passes = 0;
    while passes < budget {
        match tokio::time::timeout(Duration::from_millis(50), vdom.wait_for_work()).await {
            Ok(()) => {
                vdom.render_immediate(&mut dioxus::core::NoOpMutations);
                passes += 1;
            }
            Err(_) => break,
        }
    }
}

/// Root component: builds the controller from a recording backend + media store
/// and a coroutine that drains `Command`s into a shared `Vec`, then stashes
/// everything for the test.
#[component]
fn Harness() -> Element {
    let app_state = use_signal(EpisodeState::default);
    let playlists = use_signal(PlaylistState::default);
    let playbacks = use_signal(PlaybackOverlay::default);
    let downloads = use_signal(DownloadState::default);
    let now_playing = use_signal(|| None::<NowPlaying>);
    // Seed a logged-in config (server + token) so the stream-URL builder, which now
    // reads auth from `ClientConfig` rather than `EpisodeState`, can resolve a remote URL.
    let config = use_signal(|| {
        let mut c = ClientConfig::default();
        c.server_url = Some("https://srv".into());
        c.access_token = Some("tok".into());
        c
    });
    let sleep = use_signal(SleepState::default);
    let play_context = use_signal(PlayContext::default);
    let trigger = use_signal(|| 0u32);

    // Coroutine that collects every dispatched Command.
    let sink: Rc<RefCell<Vec<Command>>> = use_hook(|| Rc::new(RefCell::new(Vec::new())));
    let drain_sink = sink.clone();
    let dispatch = use_coroutine(move |mut rx: UnboundedReceiver<Command>| {
        let drain_sink = drain_sink.clone();
        async move {
            use futures::StreamExt;
            while let Some(cmd) = rx.next().await {
                drain_sink.borrow_mut().push(cmd);
            }
        }
    });

    let recorder = Recorder::default();
    let media = Rc::new(RecordingMediaStore::default());

    use_hook({
        let recorder = recorder.clone();
        let media = media.clone();
        let sink = sink.clone();
        move || {
            let backend: Box<dyn PlayerBackend> = Box::new(RecordingBackend(recorder.clone()));
            let controller = PlayerController::new(
                backend,
                app_state,
                playlists,
                playbacks,
                downloads,
                now_playing,
                dispatch,
                Some(media.clone() as Rc<dyn MediaStore>),
                config,
                sleep,
                play_context,
            );
            CONTROLLER.with(|c| *c.borrow_mut() = Some(controller));
            APP_STATE.with(|c| *c.borrow_mut() = Some(app_state));
            PLAYLISTS.with(|c| *c.borrow_mut() = Some(playlists));
            DOWNLOADS.with(|c| *c.borrow_mut() = Some(downloads));
            NOW_PLAYING.with(|c| *c.borrow_mut() = Some(now_playing));
            COMMANDS.with(|c| *c.borrow_mut() = Some(sink.clone()));
            RECORDER.with(|c| *c.borrow_mut() = Some(recorder.clone()));
            MEDIA.with(|c| *c.borrow_mut() = Some(media.clone()));
            CONFIG.with(|c| *c.borrow_mut() = Some(config));
            SCOPE_BOUND.with(|c| *c.borrow_mut() = Some(ScopeBound::capture()));
            TRIGGER.with(|c| *c.borrow_mut() = Some(trigger));
        }
    });

    // Drain queued scope-requiring actions whenever the trigger bumps. Running
    // here (inside this scope) is what lets the controller's internal `spawn`
    // find a current scope.
    use_effect(move || {
        let _subscribe = trigger();
        let pending: Vec<Action> = ACTIONS.with(|c| std::mem::take(&mut *c.borrow_mut()));
        let c = CONTROLLER.with(|c| c.borrow().clone());
        let Some(c) = c else { return };
        for action in pending {
            match action {
                Action::RequestPlay(id) => c.request_play(id),
                Action::RequestPlayIn(id, ctx) => c.request_play_in(id, ctx),
                Action::DownloadAndPlay(id) => c.download_and_play(id),
                Action::NextTrack => c.play_next_episode(),
                Action::PreviousTrack => c.play_previous_episode(),
                Action::Play => c.play(),
                Action::Toggle => c.toggle(),
                Action::Tick => c.tick(),
            }
        }
    });

    rsx! {
        div {}
    }
}

/// Build + mount the harness vdom and return it ready to drive.
async fn mount() -> VirtualDom {
    reset_harness();
    let mut vdom = VirtualDom::new(Harness);
    vdom.rebuild(&mut dioxus::core::NoOpMutations);
    // Let the drain coroutine spin up (its future is spawned by use_future).
    pump(&mut vdom, 5).await;
    vdom
}

// ── State-building helpers (driven inside vdom.in_runtime) ───────────────────

fn ep(id: i32, podcast_id: i32, status: DownloadStatus) -> EpisodeData {
    let ts = Utc::now();
    EpisodeData {
        id,
        podcast_id,
        title: format!("Episode {id}"),
        description: None,
        content_url: String::new(),
        guid: None,
        art_url: None,
        published_at: Some(ts),
        downloaded_at: None,
        content_file_path: None,
        download_size: None,
        art_file_path: None,
        download_status: status,
        download_started_at: None,
        download_attempts: 0,
        playback_status: PlaybackStatus::Unplayed,
        duration_secs: None,
        created_at: ts,
        updated_at: ts,
        podcast: None,
        playback: None,
        chapters: None,
    }
}

fn playlist(id: i32, is_default: bool) -> PlaylistData {
    let ts = Utc::now();
    PlaylistData {
        id,
        name: format!("Playlist {id}"),
        description: None,
        is_default,
        position: 0,
        on_remove_delete_file_server: false,
        on_remove_delete_file_client: false,
        created_at: ts,
        updated_at: ts,
        episode_ids: None,
        episode_playlist: None,
    }
}

/// Set the playback preference into the config signal.
fn set_pref(vdom: &mut VirtualDom, pref: PlaybackPreference) {
    let mut cfg = config_sig();
    vdom.in_runtime(|| {
        let mut c = cfg.peek().clone();
        c.playback_prefs.playback_preference = pref;
        cfg.set(c);
    });
}

/// Mark the config as embedded-server mode (forces StreamOnly at the
/// controller's preference chokepoint).
fn set_embedded(vdom: &mut VirtualDom) {
    let mut cfg = config_sig();
    vdom.in_runtime(|| {
        let mut c = cfg.peek().clone();
        c.server_kind = ServerKind::Embedded;
        cfg.set(c);
    });
}

/// Toggle the auto-advance-to-next-in-queue setting (defaults on).
fn set_auto_advance(vdom: &mut VirtualDom, on: bool) {
    let mut cfg = config_sig();
    vdom.in_runtime(|| {
        let mut c = cfg.peek().clone();
        c.playback_prefs.auto_advance = on;
        cfg.set(c);
    });
}

/// Replace the whole EpisodeState + DownloadState (the worker-owned pair the player
/// reads). Bundled so the matrix/nav helpers can seed both in one call.
fn set_state(vdom: &mut VirtualDom, state: (EpisodeState, PlaylistState, DownloadState)) {
    let (app, playlists, downloads) = state;
    let mut sig = app_state_sig();
    let mut psig = playlists_sig();
    let mut dsig = downloads_sig();
    vdom.in_runtime(move || {
        sig.set(app);
        psig.set(playlists);
        dsig.set(downloads);
    });
}

/// Current now_playing state (peeked outside the runtime via read).
fn np_state(vdom: &mut VirtualDom) -> Option<PlaybackState> {
    let sig = now_playing_sig();
    vdom.in_runtime(|| sig.peek().as_ref().map(|n| n.state.clone()))
}

fn count_download_to_device(cmds: &[Command]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, Command::DownloadToDevice { .. }))
        .count()
}

fn has_stream_load(recorder: &Recorder) -> bool {
    recorder
        .calls()
        .iter()
        .any(|c| matches!(c, BackendCall::Load { remote: true, .. }))
}

fn has_local_load(recorder: &Recorder) -> bool {
    recorder
        .calls()
        .iter()
        .any(|c| matches!(c, BackendCall::Load { remote: false, .. }))
}

// ════════════════════════════════════════════════════════════════════════════
// request_play preference matrix
// ════════════════════════════════════════════════════════════════════════════

/// A state where episode 1 (podcast 7) is server-Downloaded, with the given
/// client-device state and the device bytes optionally present.
fn state_for(device: Option<ClientDownloadState>) -> (EpisodeState, PlaylistState, DownloadState) {
    let mut s = EpisodeState::default();
    s.episodes_by_id
        .insert(1, ep(1, 7, DownloadStatus::Downloaded));
    let mut d = DownloadState::default();
    if let Some(dev) = device {
        d.client_downloads.insert(1, dev);
    }
    (s, PlaylistState::default(), d)
}

#[tokio::test]
async fn request_play_download_only_matrix() {
    // ── Downloaded + bytes present → play the device copy (local load) ──
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
        media().present.borrow_mut().push(1);
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(
            has_local_load(&recorder()),
            "DownloadOnly + Downloaded should play the device copy"
        );
        assert_eq!(
            count_download_to_device(&commands()),
            0,
            "no download command when bytes are present"
        );
    }

    // ── Downloading → Preparing, no command ──
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloading)));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert_eq!(np_state(&mut vdom), Some(PlaybackState::Preparing));
        assert_eq!(count_download_to_device(&commands()), 0);
    }

    // ── Failed → DownloadToDevice + Preparing ──
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Failed)));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert_eq!(np_state(&mut vdom), Some(PlaybackState::Preparing));
        assert_eq!(count_download_to_device(&commands()), 1);
    }

    // ── None (no device entry) → DownloadToDevice + Preparing ──
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
        set_state(&mut vdom, state_for(None));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert_eq!(np_state(&mut vdom), Some(PlaybackState::Preparing));
        assert_eq!(count_download_to_device(&commands()), 1);
        assert!(!has_stream_load(&recorder()), "DownloadOnly never streams");
    }
}

/// `download_and_play` forces the fetch-to-device-then-play path even when the
/// playback preference would otherwise stream — the explicit "Download & Play"
/// action on the detail page.
#[tokio::test]
async fn download_and_play_forces_download_regardless_of_preference() {
    // StreamOnly pref, nothing local → still downloads + Preparing, never streams.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamOnly);
        set_state(&mut vdom, state_for(None));
        run_action(&mut vdom, Action::DownloadAndPlay(1), 20).await;
        assert_eq!(np_state(&mut vdom), Some(PlaybackState::Preparing));
        assert_eq!(count_download_to_device(&commands()), 1);
        assert!(
            !has_stream_load(&recorder()),
            "Download & Play never streams"
        );
        // The gesture path must prime the backend: the real play() fires when
        // the download lands, outside the click's user-activation window, and
        // browser autoplay policy rejects it on a fresh page otherwise (the
        // "freshly downloaded episode won't play" bug).
        assert!(
            recorder().calls().contains(&BackendCall::Prime),
            "entering Preparing must prime the backend inside the gesture"
        );
    }

    // Already on device → play the local copy, no download command.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamOnly);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
        media().present.borrow_mut().push(1);
        run_action(&mut vdom, Action::DownloadAndPlay(1), 20).await;
        assert!(has_local_load(&recorder()), "device copy plays locally");
        assert_eq!(count_download_to_device(&commands()), 0);
    }

    // Mid-download → Preparing, no duplicate command.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamFallback);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloading)));
        run_action(&mut vdom, Action::DownloadAndPlay(1), 20).await;
        assert_eq!(np_state(&mut vdom), Some(PlaybackState::Preparing));
        assert_eq!(count_download_to_device(&commands()), 0);
    }
}

#[tokio::test]
async fn request_play_stream_first_and_download_matrix() {
    // Downloaded → play device copy, no download command.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamFirstAndDownload);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
        media().present.borrow_mut().push(1);
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_local_load(&recorder()));
        assert_eq!(count_download_to_device(&commands()), 0);
    }

    // Not downloaded → DownloadToDevice AND stream now.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamFirstAndDownload);
        set_state(&mut vdom, state_for(None));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert_eq!(
            count_download_to_device(&commands()),
            1,
            "background download"
        );
        assert!(has_stream_load(&recorder()), "and stream immediately");
    }
}

#[tokio::test]
async fn request_play_stream_fallback_matrix() {
    // Downloaded → play device copy, no stream, no download.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamFallback);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
        media().present.borrow_mut().push(1);
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_local_load(&recorder()));
        assert!(!has_stream_load(&recorder()));
        assert_eq!(count_download_to_device(&commands()), 0);
    }

    // Not downloaded → stream, NO download command.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamFallback);
        set_state(&mut vdom, state_for(None));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_stream_load(&recorder()));
        assert_eq!(
            count_download_to_device(&commands()),
            0,
            "fallback never downloads"
        );
    }
}

#[tokio::test]
async fn request_play_stream_only_always_streams_never_downloads() {
    // Even when a device copy exists, StreamOnly streams and never downloads.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamOnly);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
        media().present.borrow_mut().push(1);
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_stream_load(&recorder()), "StreamOnly streams");
        assert!(
            !has_local_load(&recorder()),
            "StreamOnly ignores the device copy"
        );
        assert_eq!(count_download_to_device(&commands()), 0);
    }

    // No device copy → still just streams.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::StreamOnly);
        set_state(&mut vdom, state_for(None));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_stream_load(&recorder()));
        assert_eq!(count_download_to_device(&commands()), 0);
    }
}

/// Embedded-server mode forces StreamOnly at the controller's preference
/// chokepoint regardless of the STORED preference (which is preserved for a
/// later switch back to remote): play streams from the built-in server and the
/// device pipeline never fires — even under DownloadOnly, whose remote-mode
/// invariant is the exact opposite.
#[tokio::test]
async fn request_play_embedded_forces_stream_only() {
    // DownloadOnly stored, embedded active, nothing local → streams, no download.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
        set_embedded(&mut vdom);
        set_state(&mut vdom, state_for(None));
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_stream_load(&recorder()), "embedded always streams");
        assert_eq!(
            count_download_to_device(&commands()),
            0,
            "no device pipeline in embedded mode"
        );
    }

    // A stale device copy from a previous remote life is ignored — embedded
    // streams the server copy.
    {
        let mut vdom = mount().await;
        set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
        set_embedded(&mut vdom);
        set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
        media().present.borrow_mut().push(1);
        run_action(&mut vdom, Action::RequestPlay(1), 20).await;
        assert!(has_stream_load(&recorder()), "embedded streams");
        assert_eq!(count_download_to_device(&commands()), 0);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// adjacent_episode / has_adjacent / next_track / previous_track
// ════════════════════════════════════════════════════════════════════════════

/// State with a default queue [10, 20, 30] and a podcast list [40, 50] (podcast 7).
fn nav_state() -> (EpisodeState, PlaylistState, DownloadState) {
    let mut s = EpisodeState::default();
    let mut p = PlaylistState::default();
    p.playlists.push(playlist(1, true));
    p.episodes_by_playlist.insert(1, vec![10, 20, 30]);
    for id in [10, 20, 30] {
        s.episodes_by_id
            .insert(id, ep(id, 7, DownloadStatus::Downloaded));
    }
    s.episodes_by_podcast.insert(7, vec![40, 50]);
    for id in [40, 50] {
        s.episodes_by_id
            .insert(id, ep(id, 7, DownloadStatus::Downloaded));
    }
    // `queue_id()` reads the cached `QueueState`, not the playlists pool — derive it
    // from the seeded default playlist so the queue resolves (as the worker does).
    p.recompute_queue(true);
    (s, p, DownloadState::default())
}

/// Force now_playing to a given episode in a Playing state.
fn set_now_playing(vdom: &mut VirtualDom, episode_id: i32) {
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id,
            state: PlaybackState::Playing,
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: false,
        }))
    });
}

#[tokio::test]
async fn adjacent_uses_queue_then_podcast_order() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());

    // Nothing playing → no neighbors.
    assert!(!vdom.in_runtime(|| controller().has_adjacent(1)));
    assert!(!vdom.in_runtime(|| controller().has_adjacent(-1)));

    // Playing 20 (middle of the queue): next 30, prev 10.
    set_now_playing(&mut vdom, 20);
    assert!(vdom.in_runtime(|| controller().has_adjacent(1)));
    assert!(vdom.in_runtime(|| controller().has_adjacent(-1)));

    // Ends of the queue.
    set_now_playing(&mut vdom, 10);
    assert!(
        !vdom.in_runtime(|| controller().has_adjacent(-1)),
        "no prev before first"
    );
    assert!(vdom.in_runtime(|| controller().has_adjacent(1)));
    set_now_playing(&mut vdom, 30);
    assert!(
        !vdom.in_runtime(|| controller().has_adjacent(1)),
        "no next after last"
    );

    // Playing 40 (NOT in the queue → falls back to podcast order [40, 50]).
    set_now_playing(&mut vdom, 40);
    assert!(
        vdom.in_runtime(|| controller().has_adjacent(1)),
        "podcast next exists"
    );
    assert!(
        !vdom.in_runtime(|| controller().has_adjacent(-1)),
        "no prev before podcast first"
    );
    set_now_playing(&mut vdom, 50);
    assert!(
        !vdom.in_runtime(|| controller().has_adjacent(1)),
        "no next after podcast last"
    );
}

#[tokio::test]
async fn next_track_requests_play_on_neighbor() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly); // simplest path: streams the neighbor
    set_state(&mut vdom, nav_state());
    set_now_playing(&mut vdom, 20);

    run_action(&mut vdom, Action::NextTrack, 20).await;
    // Neighbor is 30 → now_playing becomes 30 (Loading after the stream load).
    let id = vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id));
    assert_eq!(id, Some(30), "next_track played the queue neighbor");
    assert!(has_stream_load(&recorder()));
}

#[tokio::test]
async fn previous_track_at_start_is_noop() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, nav_state());
    set_now_playing(&mut vdom, 10); // first in queue

    run_action(&mut vdom, Action::PreviousTrack, 20).await;
    // Still on 10, nothing loaded (no neighbor).
    let id = vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id));
    assert_eq!(id, Some(10));
    assert!(!has_stream_load(&recorder()), "no neighbor → no load");
}

// ════════════════════════════════════════════════════════════════════════════
// Local-error retry: transient device-copy load failures (iOS WebKit's
// freshly-committed IndexedDB blob can fail to load, then read fine moments
// later — the controller re-resolves the source before surfacing an error)
// ════════════════════════════════════════════════════════════════════════════

fn count_local_loads(recorder: &Recorder) -> usize {
    recorder
        .calls()
        .iter()
        .filter(|c| matches!(c, BackendCall::Load { remote: false, .. }))
        .count()
}

/// Mount + start a device-copy playback of episode 1 (DownloadOnly, bytes
/// present): the first local load is issued and the player sits in `Loading`.
async fn mount_local_loading() -> VirtualDom {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
    set_state(&mut vdom, state_for(Some(ClientDownloadState::Downloaded)));
    media().present.borrow_mut().push(1);
    run_action(&mut vdom, Action::RequestPlay(1), 20).await;
    assert!(
        has_local_load(&recorder()),
        "precondition: device load issued"
    );
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Loading));
    vdom
}

/// A device load's backend error is retried with a FRESH source resolution
/// (fresh `audio_url` → fresh load) instead of latching `Error`; straggler
/// events during the retry window are discarded; the retried load then starts
/// normally.
#[tokio::test]
async fn local_load_error_retries_with_fresh_source_before_surfacing() {
    let mut vdom = mount_local_loading().await;

    recorder().queue_events(vec![PlayerEvent::Error("audio error (code 4)".into())]);
    run_action(&mut vdom, Action::Tick, 5).await;
    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Loading),
        "first device-load failure is retried, not surfaced"
    );

    // Stragglers of the failed load (the late play-promise rejection, the
    // element re-reporting its error every poll) must not burn a second retry
    // or latch Error while the retry is pending.
    recorder().queue_events(vec![PlayerEvent::Error(
        "playback couldn't start (NotSupportedError)".into(),
    )]);
    run_action(&mut vdom, Action::Tick, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Loading));

    // The first backoff (750ms) elapses → the retry re-resolves and re-loads.
    tokio::time::sleep(Duration::from_millis(950)).await;
    pump(&mut vdom, 20).await;
    assert_eq!(
        count_local_loads(&recorder()),
        2,
        "retry issued a fresh device load"
    );
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Loading));

    // The fresh load starts: first clock tick flips to Playing as usual.
    recorder().queue_events(vec![PlayerEvent::TimeUpdate(1.0)]);
    run_action(&mut vdom, Action::Tick, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Playing));
}

/// The retry budget is bounded: a device copy that keeps failing surfaces the
/// error after the ladder (two retries) is spent.
#[tokio::test]
async fn local_load_error_budget_exhausted_surfaces_error() {
    let mut vdom = mount_local_loading().await;

    // Attempt 1 fails → retry #1 after 750ms.
    recorder().queue_events(vec![PlayerEvent::Error("audio error (code 4)".into())]);
    run_action(&mut vdom, Action::Tick, 5).await;
    tokio::time::sleep(Duration::from_millis(950)).await;
    pump(&mut vdom, 20).await;
    assert_eq!(count_local_loads(&recorder()), 2);

    // Attempt 2 fails → retry #2 after 2s.
    recorder().queue_events(vec![PlayerEvent::Error("audio error (code 4)".into())]);
    run_action(&mut vdom, Action::Tick, 5).await;
    tokio::time::sleep(Duration::from_millis(2_200)).await;
    pump(&mut vdom, 20).await;
    assert_eq!(count_local_loads(&recorder()), 3);

    // Attempt 3 fails → budget spent, the error finally latches.
    recorder().queue_events(vec![PlayerEvent::Error("audio error (code 4)".into())]);
    run_action(&mut vdom, Action::Tick, 5).await;
    assert!(
        matches!(np_state(&mut vdom), Some(PlaybackState::Error(_))),
        "failure after the retry budget latches Error"
    );
}

/// A fresh user play during the retry window supersedes the scheduled retry
/// (generation guard): the new load proceeds and the stale retry never fires a
/// surplus load.
#[tokio::test]
async fn local_retry_superseded_by_new_play_never_fires() {
    let mut vdom = mount_local_loading().await;

    recorder().queue_events(vec![PlayerEvent::Error("audio error (code 4)".into())]);
    run_action(&mut vdom, Action::Tick, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Loading));

    // User taps play again before the backoff elapses → immediate fresh load.
    run_action(&mut vdom, Action::RequestPlay(1), 20).await;
    assert_eq!(count_local_loads(&recorder()), 2);

    // The superseded retry's timer expires without adding a third load.
    tokio::time::sleep(Duration::from_millis(950)).await;
    pump(&mut vdom, 20).await;
    assert_eq!(
        count_local_loads(&recorder()),
        2,
        "stale retry aborted on the generation guard"
    );
}

/// Stream-source errors are NOT retried — they surface immediately, exactly as
/// before the local retry existed.
#[tokio::test]
async fn stream_load_error_surfaces_immediately_without_retry() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, state_for(None));
    run_action(&mut vdom, Action::RequestPlay(1), 20).await;
    assert!(
        has_stream_load(&recorder()),
        "precondition: stream load issued"
    );

    recorder().queue_events(vec![PlayerEvent::Error("network gave up".into())]);
    run_action(&mut vdom, Action::Tick, 5).await;
    assert!(
        matches!(np_state(&mut vdom), Some(PlaybackState::Error(_))),
        "stream errors surface immediately"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// tick(): Ended latch, Loading→Playing, SetCursor cadence, early-returns
// ════════════════════════════════════════════════════════════════════════════

fn count_mark_played(cmds: &[Command]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, Command::MarkPlayed { played: true, .. }))
        .count()
}

#[tokio::test]
async fn tick_ended_latch_marks_played_once() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    // Disable auto-advance so this isolates the Ended latch (20 is mid-queue, so
    // auto-advance would otherwise move on to 30 instead of settling on Ended —
    // that path is covered by `tick_ended_auto_advances_to_next_in_queue`).
    set_auto_advance(&mut vdom, false);
    set_now_playing(&mut vdom, 20);

    // First tick sees an Ended event.
    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Ended));
    assert_eq!(count_mark_played(&commands()), 1, "exactly one MarkPlayed");

    // Subsequent ticks early-return on the Ended latch even if the backend keeps
    // reporting Ended — no repeat MarkPlayed.
    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;
    assert_eq!(
        count_mark_played(&commands()),
        1,
        "Ended latch prevents repeats"
    );
}

#[tokio::test]
async fn tick_ended_auto_advances_to_next_in_queue() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly); // simplest path: streams the next
    set_state(&mut vdom, nav_state());
    set_auto_advance(&mut vdom, true);
    set_now_playing(&mut vdom, 20); // mid-queue [10, 20, 30] → next is 30

    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 20).await; // request_play resolves the source over several pumps

    // The finished episode is still marked played exactly once...
    assert_eq!(
        count_mark_played(&commands()),
        1,
        "ended episode marked played"
    );
    // ...and playback advanced to its queue neighbor rather than latching Ended.
    let id = vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id));
    assert_eq!(id, Some(30), "auto-advanced to the next queue item");
    assert!(has_stream_load(&recorder()));
}

#[tokio::test]
async fn tick_ended_at_queue_tail_settles_ended_even_with_auto_advance() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    set_auto_advance(&mut vdom, true);
    set_now_playing(&mut vdom, 30); // last in the queue → no next_up

    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;

    // No next in the queue → stays on 30 and settles on the terminal Ended state.
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Ended));
    let id = vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id));
    assert_eq!(id, Some(30), "no advance past the end of the queue");
}

// ════════════════════════════════════════════════════════════════════════════
// Play context: playlist-aware continuation
// ════════════════════════════════════════════════════════════════════════════

/// [`nav_state`] plus a non-default playlist 2 → [20, 40]: episode 20 is also
/// mid-queue ([10, 20, 30]), 40 is only in the playlist (and podcast order) —
/// so queue vs playlist continuation from 20 is observable (30 vs 40).
fn context_nav_state() -> (EpisodeState, PlaylistState, DownloadState) {
    let (s, mut p, d) = nav_state();
    p.playlists.push(playlist(2, false));
    p.episodes_by_playlist.insert(2, vec![20, 40]);
    (s, p, d)
}

fn now_playing_id(vdom: &mut VirtualDom) -> Option<i32> {
    vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id))
}

fn play_context(vdom: &mut VirtualDom) -> Option<i32> {
    vdom.in_runtime(|| controller().play_context())
}

/// The core fix: playing FROM a playlist makes that playlist the continuation
/// source — auto-advance follows it (not the queue), and the context survives
/// the track change so the playlist keeps playing through.
#[tokio::test]
async fn play_in_context_auto_advances_within_playlist() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, context_nav_state());
    set_auto_advance(&mut vdom, true);

    run_action(&mut vdom, Action::RequestPlayIn(20, Some(2)), 20).await;
    assert_eq!(play_context(&mut vdom), Some(2), "context set by the play");
    set_now_playing(&mut vdom, 20); // force Playing so the Ended event is drained

    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 20).await;

    assert_eq!(
        now_playing_id(&mut vdom),
        Some(40),
        "advanced within playlist 2 ([20, 40]), not to the queue's 30"
    );
    assert_eq!(
        play_context(&mut vdom),
        Some(2),
        "context survives auto-advance"
    );
}

/// End of the context playlist → playback stops (terminal Ended), no queue
/// fallback — the same rule as the queue's own tail.
#[tokio::test]
async fn play_in_context_stops_at_playlist_tail() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, context_nav_state());
    set_auto_advance(&mut vdom, true);

    run_action(&mut vdom, Action::RequestPlayIn(40, Some(2)), 20).await;
    set_now_playing(&mut vdom, 40); // last in playlist 2

    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;

    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Ended));
    assert_eq!(
        now_playing_id(&mut vdom),
        Some(40),
        "no queue fallback past the playlist's end"
    );
}

/// A contextless play (`request_play_in(id, None)` — e.g. the episode detail
/// page) resets the context, restoring queue continuation.
#[tokio::test]
async fn play_in_none_resets_to_queue_semantics() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, context_nav_state());
    set_auto_advance(&mut vdom, true);

    run_action(&mut vdom, Action::RequestPlayIn(20, Some(2)), 20).await;
    run_action(&mut vdom, Action::RequestPlayIn(20, None), 20).await;
    assert_eq!(
        play_context(&mut vdom),
        None,
        "context reset by a None play"
    );
    set_now_playing(&mut vdom, 20);

    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 20).await;

    assert_eq!(
        now_playing_id(&mut vdom),
        Some(30),
        "queue continuation restored ([10, 20, 30])"
    );
}

/// The transport Next/Prev buttons follow the context playlist too.
#[tokio::test]
async fn transport_next_and_previous_follow_context_playlist() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, context_nav_state());

    run_action(&mut vdom, Action::RequestPlayIn(20, Some(2)), 20).await;
    set_now_playing(&mut vdom, 20);
    run_action(&mut vdom, Action::NextTrack, 20).await;
    assert_eq!(
        now_playing_id(&mut vdom),
        Some(40),
        "Next follows playlist 2, not the queue"
    );

    set_now_playing(&mut vdom, 40);
    run_action(&mut vdom, Action::PreviousTrack, 20).await;
    assert_eq!(
        now_playing_id(&mut vdom),
        Some(20),
        "Prev walks back within playlist 2"
    );
}

/// `stop()` (closing the player) ends the listening session and clears the
/// context, like it clears the sleep timer.
#[tokio::test]
async fn stop_clears_play_context() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, context_nav_state());

    run_action(&mut vdom, Action::RequestPlayIn(20, Some(2)), 20).await;
    assert_eq!(play_context(&mut vdom), Some(2));

    vdom.in_runtime(|| controller().stop());
    pump(&mut vdom, 5).await;
    assert_eq!(play_context(&mut vdom), None, "stop() clears the context");
}

/// A context playlist deleted mid-play (its membership gone from the local
/// state) degrades gracefully to queue semantics on the next advance.
#[tokio::test]
async fn deleted_context_playlist_degrades_to_queue() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, context_nav_state());
    set_auto_advance(&mut vdom, true);

    run_action(&mut vdom, Action::RequestPlayIn(20, Some(2)), 20).await;
    // Playlist 2 disappears (deleted/unloaded) — back to the plain nav state.
    set_state(&mut vdom, nav_state());
    set_now_playing(&mut vdom, 20);

    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 20).await;

    assert_eq!(
        now_playing_id(&mut vdom),
        Some(30),
        "unknown context falls back to the queue neighbor"
    );
}

#[tokio::test]
async fn tick_early_returns_for_preparing_and_none() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());

    // None: nothing playing → poll_events must not even be drained.
    recorder().queue_events(vec![PlayerEvent::Ended]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;
    assert_eq!(count_mark_played(&commands()), 0);
    assert_eq!(np_state(&mut vdom), None);
    // The queued batch is still unconsumed (tick returned before polling).
    assert_eq!(recorder().events.borrow().len(), 1);

    // Preparing: also early-returns (no stale-element bleed).
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id: 20,
            state: PlaybackState::Preparing,
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: true,
        }))
    });
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Preparing));
    assert_eq!(recorder().events.borrow().len(), 1, "Preparing didn't poll");
}

#[tokio::test]
async fn tick_time_update_flips_loading_to_playing_and_clears_buffering() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    // Start in Loading + buffering (as load_and_play does).
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id: 20,
            state: PlaybackState::Loading,
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: true,
        }))
    });

    recorder().queue_events(vec![PlayerEvent::TimeUpdate(3.5)]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;

    let np = vdom.in_runtime(|| now_playing_sig().peek().clone());
    let np = np.unwrap();
    assert_eq!(
        np.state,
        PlaybackState::Playing,
        "first clock tick → Playing"
    );
    assert!(!np.buffering, "buffering cleared");
    assert!((np.position_secs - 3.5).abs() < 1e-6);
}

#[tokio::test]
async fn tick_set_cursor_only_on_mult_of_40_while_playing() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());

    // Playing: a TimeUpdate per tick keeps it Playing; cursor persists at the
    // 40th tick only.
    set_now_playing(&mut vdom, 20);
    for _ in 0..39 {
        recorder().queue_events(vec![PlayerEvent::TimeUpdate(1.0)]);
        vdom.in_runtime(|| controller().tick());
        pump(&mut vdom, 2).await;
    }
    let set_cursor_before = commands()
        .iter()
        .filter(|c| matches!(c, Command::SetCursor { .. }))
        .count();
    assert_eq!(
        set_cursor_before, 0,
        "no SetCursor before the 40th playing tick"
    );

    recorder().queue_events(vec![PlayerEvent::TimeUpdate(1.0)]);
    vdom.in_runtime(|| controller().tick()); // 40th
    pump(&mut vdom, 2).await;
    let set_cursor_after = commands()
        .iter()
        .filter(|c| matches!(c, Command::SetCursor { .. }))
        .count();
    assert_eq!(
        set_cursor_after, 1,
        "SetCursor fires on the 40th playing tick"
    );
}

#[tokio::test]
async fn tick_no_set_cursor_while_paused() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    // Paused state: ticks poll (it's not None/Ended/Preparing) but the cursor
    // cadence counter only advances while Playing.
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id: 20,
            state: PlaybackState::Paused,
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: false,
        }))
    });
    for _ in 0..60 {
        // Buffering events are harmless and keep state Paused.
        recorder().queue_events(vec![PlayerEvent::Buffering(false)]);
        vdom.in_runtime(|| controller().tick());
        pump(&mut vdom, 2).await;
    }
    let set_cursor = commands()
        .iter()
        .filter(|c| matches!(c, Command::SetCursor { .. }))
        .count();
    assert_eq!(set_cursor, 0, "paused ticks never persist a cursor");
}

// ════════════════════════════════════════════════════════════════════════════
// OS-initiated pause/resume reconciliation (audio-route change, interruptions)
// ════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn external_pause_event_syncs_state_to_paused_and_persists() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    // Playing, with a real position so the persisted cursor is meaningful.
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id: 20,
            state: PlaybackState::Playing,
            position_secs: 42.0,
            duration_secs: Some(100.0),
            rate: 1.0,
            buffering: false,
        }))
    });

    // The OS paused the element on its own (e.g. a speaker disconnected). The
    // backend surfaces this as a `Paused` edge.
    recorder().queue_events(vec![PlayerEvent::Paused]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Paused),
        "an OS-initiated pause must flip the controller out of Playing"
    );
    // The cursor is persisted on the external-pause edge, like a real pause.
    assert!(
        commands()
            .iter()
            .any(|c| matches!(c, Command::SetCursor { episode_id: 20, cursor } if *cursor == 42)),
        "external pause should persist the cursor"
    );
}

#[tokio::test]
async fn external_resume_event_syncs_state_back_to_playing() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    // Paused (e.g. just got externally paused).
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id: 20,
            state: PlaybackState::Paused,
            position_secs: 42.0,
            duration_secs: Some(100.0),
            rate: 1.0,
            buffering: false,
        }))
    });

    // The OS handed audio focus back and the element resumed on its own.
    recorder().queue_events(vec![PlayerEvent::Playing]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Playing),
        "an OS-initiated resume must flip the controller back to Playing"
    );
}

#[tokio::test]
async fn external_playing_edge_does_not_disturb_loading() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    // Loading: the element's `play()` has resolved (paused=false → a `Playing`
    // edge) but the first clock tick hasn't arrived. The transition to Playing is
    // owned by `TimeUpdate`, so a bare `Playing` edge must NOT flip it early.
    let mut sig = now_playing_sig();
    vdom.in_runtime(move || {
        sig.set(Some(NowPlaying {
            episode_id: 20,
            state: PlaybackState::Loading,
            position_secs: 0.0,
            duration_secs: None,
            rate: 1.0,
            buffering: true,
        }))
    });

    recorder().queue_events(vec![PlayerEvent::Playing]);
    vdom.in_runtime(|| controller().tick());
    pump(&mut vdom, 5).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Loading),
        "a Playing edge while Loading must not pre-empt the TimeUpdate transition"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// update_now_playing no-op guard
// ════════════════════════════════════════════════════════════════════════════

/// Probe component that subscribes to now_playing and bumps a render counter.
#[component]
fn NpProbe() -> Element {
    NP_RENDERS.with(|c| c.set(c.get() + 1));
    let np = now_playing_sig();
    let _subscribe = np.read().is_some();
    rsx! {
        div {}
    }
}

#[tokio::test]
async fn update_now_playing_noop_when_nothing_playing() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());

    // Assert via the signal itself — pause()/seek_to()/set_rate() must leave
    // now_playing None (the peek-guard returns before any notifying write).
    assert!(vdom.in_runtime(|| now_playing_sig().peek().is_none()));

    vdom.in_runtime(|| {
        let c = controller();
        c.pause();
        c.seek_to(12.0);
        c.set_rate(1.5);
    });
    pump(&mut vdom, 5).await;

    // The signal is never written (stays None) — the guard returned early before
    // taking a notifying write lock.
    assert!(
        vdom.in_runtime(|| now_playing_sig().peek().is_none()),
        "update_now_playing wrote the signal while nothing was playing"
    );
    // Seek/rate still forward to the backend (the update_now_playing peek-guard
    // only suppresses the *signal* write). `pause()` is now state-guarded, so with
    // nothing playing it's a full no-op — it never reaches the backend.
    let calls = recorder().calls();
    assert!(
        !calls.contains(&BackendCall::Pause),
        "pause() must be a no-op when nothing is playing"
    );
    assert!(calls.contains(&BackendCall::Seek(12.0)));
    assert!(calls.contains(&BackendCall::SetRate(1.5)));
}

/// Root for the render-count probe test: stashes a now_playing signal + a
/// controller AND mounts a subscriber, so we can prove the no-op guard doesn't
/// re-render now_playing subscribers.
#[component]
fn ProbeRoot() -> Element {
    let app_state = use_signal(EpisodeState::default);
    let playlists = use_signal(PlaylistState::default);
    let playbacks = use_signal(PlaybackOverlay::default);
    let downloads = use_signal(DownloadState::default);
    let now_playing = use_signal(|| None::<NowPlaying>);
    let config = use_signal(ClientConfig::default);
    let sleep = use_signal(SleepState::default);
    let play_context = use_signal(PlayContext::default);
    let dispatch = use_coroutine(|mut rx: UnboundedReceiver<Command>| async move {
        use futures::StreamExt;
        while rx.next().await.is_some() {}
    });
    use_hook(move || {
        let backend: Box<dyn PlayerBackend> = Box::new(RecordingBackend(Recorder::default()));
        let controller = PlayerController::new(
            backend,
            app_state,
            playlists,
            playbacks,
            downloads,
            now_playing,
            dispatch,
            None,
            config,
            sleep,
            play_context,
        );
        CONTROLLER.with(|c| *c.borrow_mut() = Some(controller));
        NOW_PLAYING.with(|c| *c.borrow_mut() = Some(now_playing));
    });
    rsx! {
        NpProbe {}
    }
}

/// Render-count probe variant: a dedicated vdom whose root both stashes a
/// now_playing signal AND mounts a subscriber, so we can prove the no-op guard
/// doesn't re-render now_playing subscribers.
#[tokio::test]
async fn update_now_playing_noop_does_not_renotify_subscribers() {
    reset_harness();

    let mut vdom = VirtualDom::new(ProbeRoot);
    vdom.rebuild(&mut dioxus::core::NoOpMutations);
    pump(&mut vdom, 5).await;
    let after_mount = NP_RENDERS.with(|c| c.get());

    // now_playing is None → pause()/seek_to()/set_rate() must NOT notify the
    // subscriber (the peek-guard returns before any write()).
    vdom.in_runtime(|| {
        let c = controller();
        c.pause();
        c.seek_to(5.0);
        c.set_rate(2.0);
    });
    pump(&mut vdom, 10).await;
    assert_eq!(
        NP_RENDERS.with(|c| c.get()),
        after_mount,
        "no-op update re-rendered a now_playing subscriber — the peek guard regressed"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// State-aware transport (C3): a play/pause tap must not disturb `Preparing`
// ════════════════════════════════════════════════════════════════════════════

/// A `Paused` now_playing for `id` at 30s — the one state `resume()` acts from.
fn paused_np(id: i32) -> NowPlaying {
    NowPlaying {
        episode_id: id,
        state: PlaybackState::Paused,
        position_secs: 30.0,
        duration_secs: Some(100.0),
        rate: 1.0,
        buffering: false,
    }
}

/// While a Download & Play download is in flight (`Preparing`), a tap on the
/// mini-player / lock-screen / row play control must NOT flip state: that both
/// wedges the download→play handoff (the watcher requires `Preparing`) and lets
/// the next cursor persist write 0, erasing the saved resume position.
#[tokio::test]
async fn transport_ignores_taps_while_preparing() {
    let mut vdom = mount().await;
    let mut np = now_playing_sig();
    vdom.in_runtime(move || np.set(Some(NowPlaying::preparing(7))));

    vdom.in_runtime(|| {
        let c = controller();
        c.toggle();
        c.resume();
        c.pause();
    });
    pump(&mut vdom, 5).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Preparing),
        "transport must leave Preparing untouched"
    );
    assert!(
        !commands()
            .iter()
            .any(|c| matches!(c, Command::SetCursor { .. })),
        "Preparing must never persist a (zero) cursor"
    );
    let calls = recorder().calls();
    assert!(
        !calls.contains(&BackendCall::Play),
        "no backend play while Preparing"
    );
    assert!(
        !calls.contains(&BackendCall::Pause),
        "no backend pause while Preparing"
    );
}

// ── The OS media-session `play` action (PlayerController::play) ──────────────
//
// Regression cover for the lock-screen asymmetry: `pause` accepts `Playing`, but
// `resume` is a no-op outside `Paused`, so binding the OS play button straight to
// `resume` left it DEAD from `Ended`/`Error` while pause kept working. `play()`
// gives the OS action `toggle`'s state routing.

/// `now_playing` in a terminal `Ended` state for `episode_id`.
fn ended_np(episode_id: i32) -> NowPlaying {
    NowPlaying {
        episode_id,
        state: PlaybackState::Ended,
        position_secs: 100.0,
        duration_secs: Some(100.0),
        rate: 1.0,
        buffering: false,
    }
}

/// The bug, from `Ended`: a finished episode must re-play from the OS play
/// button, not sit there silently (`resume()` would no-op forever).
#[tokio::test]
async fn media_play_replays_a_finished_episode() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly); // simplest path: streams
    set_state(&mut vdom, nav_state());
    let mut np = now_playing_sig();
    vdom.in_runtime(move || np.set(Some(ended_np(20))));

    run_action(&mut vdom, Action::Play, 20).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Loading),
        "OS play on a finished episode re-plays it"
    );
    assert!(has_stream_load(&recorder()), "a real load was issued");
}

/// The bug, from `Error`: the absorbing state. `should_poll` stops polling it and
/// `resume` refuses it, so without `play()`'s routing the OS button is dead until
/// the user re-plays from inside the app.
#[tokio::test]
async fn media_play_retries_an_errored_episode() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, nav_state());
    let mut np = now_playing_sig();
    vdom.in_runtime(move || np.set(Some(NowPlaying::error(20, "playback couldn't start"))));

    run_action(&mut vdom, Action::Play, 20).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Loading),
        "OS play retries an errored episode"
    );
    assert!(has_stream_load(&recorder()), "a real load was issued");
}

/// The ordinary case still goes through the cheap `resume` primitive — a paused
/// player must NOT be re-loaded from scratch (that would lose the position).
#[tokio::test]
async fn media_play_resumes_a_paused_episode_without_reloading() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    let mut np = now_playing_sig();
    vdom.in_runtime(move || np.set(Some(paused_np(20))));

    run_action(&mut vdom, Action::Play, 5).await;

    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Playing));
    let calls = recorder().calls();
    assert!(calls.contains(&BackendCall::Play), "resumed via play()");
    assert!(
        !calls.iter().any(|c| matches!(c, BackendCall::Load { .. })),
        "a paused resume must not re-load the source"
    );
}

/// `play()` is directional, NOT a toggle: from `Playing` it must do nothing (the
/// OS sends `pause` for that). Guards against someone "simplifying" it to
/// `toggle`, which would make the OS play button pause the episode.
#[tokio::test]
async fn media_play_from_playing_is_a_noop_not_a_pause() {
    let mut vdom = mount().await;
    set_state(&mut vdom, nav_state());
    set_now_playing(&mut vdom, 20); // Playing

    run_action(&mut vdom, Action::Play, 5).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Playing),
        "OS play on a playing episode must not pause it"
    );
    assert!(
        !recorder().calls().contains(&BackendCall::Pause),
        "play() must never pause"
    );
}

/// `Preparing` keeps its existing protection through the new entry point: the
/// download→play handoff requires the state to stay `Preparing`.
#[tokio::test]
async fn media_play_leaves_preparing_untouched() {
    let mut vdom = mount().await;
    let mut np = now_playing_sig();
    vdom.in_runtime(move || np.set(Some(NowPlaying::preparing(7))));

    run_action(&mut vdom, Action::Play, 5).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Preparing),
        "play() must not disturb Preparing"
    );
    assert!(
        !recorder().calls().contains(&BackendCall::Play),
        "no backend play while Preparing"
    );
}

// ── ScopeBound: browser-invoked handlers outside the Dioxus runtime ──────────
//
// The wasm Media Session closures are invoked by the browser with NO Dioxus
// runtime on the thread-local stack; `media_session::register` wraps each one
// via `ScopeBound` so the body re-enters the provider's runtime + scope. These
// tests drive the exact wrapped shape against the real controller from
// OUTSIDE any runtime (no `vdom.in_runtime`, no `run_action`) — the condition
// that used to panic at `play_episode`'s `spawn` ("Must be called from inside
// a Dioxus runtime") and freeze the wasm app.
//
// The spawning path is the DownloadOnly/device-copy route (`play_episode`);
// stream routes never spawn — so these seed a downloaded device copy.
//
// Untestable natively (wasm-gated, compile-verified by `just check-all` and
// exercised manually in a browser): the `Closure`/`setActionHandler` glue, the
// SESSION_GEN stale-invocation no-op, `MediaSessionHandlers::Drop` clearing,
// and `use_window_event`'s listener add/remove.

/// `nav_state` with `episode_id` device-Downloaded and its bytes present in the
/// media store — the state where `request_play` reaches `play_episode`'s spawn.
fn nav_state_with_device_copy(episode_id: i32) -> (EpisodeState, PlaylistState, DownloadState) {
    let (s, p, mut d) = nav_state();
    d.client_downloads
        .insert(episode_id, ClientDownloadState::Downloaded);
    media().present.borrow_mut().push(episode_id);
    (s, p, d)
}

/// THE reported bug: lock-screen next-track (with `media_next_prev_seek` off)
/// from the browser's own event loop must advance the queue, not panic.
#[tokio::test]
async fn media_handler_next_track_outside_runtime_advances_queue() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
    let state = nav_state_with_device_copy(30);
    set_state(&mut vdom, state);
    set_now_playing(&mut vdom, 20);

    // What media_session::register's `bind` produces for "nexttrack".
    let c = controller();
    let mut handler = scope_bound().bind(move || c.play_next_episode());
    // Browser-style invocation: nothing on the runtime/scope stacks.
    handler();
    pump(&mut vdom, 20).await;

    let id = vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id));
    assert_eq!(id, Some(30), "next-track advanced to the queue neighbor");
    assert!(has_local_load(&recorder()), "the device copy was loaded");
}

/// The latent sibling: lock-screen `play` after an episode ended routes through
/// `request_play` (the same spawning path) — must replay, not panic.
#[tokio::test]
async fn media_handler_play_from_ended_outside_runtime_replays() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
    let state = nav_state_with_device_copy(20);
    set_state(&mut vdom, state);
    let mut np = now_playing_sig();
    vdom.in_runtime(move || np.set(Some(ended_np(20))));

    let c = controller();
    let mut handler = scope_bound().bind(move || c.play());
    handler();
    pump(&mut vdom, 20).await;

    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Loading),
        "OS play replayed the finished episode from outside the runtime"
    );
    assert!(has_local_load(&recorder()), "the device copy was loaded");
}

/// The re-entry must also nest cleanly when a runtime IS already active but no
/// scope is (the documented bare-`in_runtime` panic condition) — and leave the
/// stacks balanced so the vdom keeps working afterwards.
#[tokio::test]
async fn scope_bound_nests_under_bare_in_runtime() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::DownloadOnly);
    let state = nav_state_with_device_copy(30);
    set_state(&mut vdom, state);
    set_now_playing(&mut vdom, 20);

    let c = controller();
    let mut handler = scope_bound().bind(move || c.play_next_episode());
    // Runtime present, scope stack empty — `spawn` would panic here unwrapped.
    vdom.in_runtime(|| handler());
    pump(&mut vdom, 20).await;

    let id = vdom.in_runtime(|| now_playing_sig().peek().as_ref().map(|n| n.episode_id));
    assert_eq!(id, Some(30), "nested re-entry still advanced the queue");
    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Loading),
        "vdom stayed healthy after the nested runtime entry"
    );
}

/// `toggle` now delegates to `pause`/`play`; assert its full state matrix is
/// unchanged by that refactor.
#[tokio::test]
async fn toggle_matrix_survives_delegating_to_play_and_pause() {
    let mut vdom = mount().await;
    set_pref(&mut vdom, PlaybackPreference::StreamOnly);
    set_state(&mut vdom, nav_state());
    let mut np = now_playing_sig();

    // Playing → Paused.
    set_now_playing(&mut vdom, 20);
    run_action(&mut vdom, Action::Toggle, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Paused));

    // Paused → Playing.
    run_action(&mut vdom, Action::Toggle, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Playing));

    // Loading → Paused (a tap cancels autoplay).
    vdom.in_runtime(move || np.set(Some(NowPlaying::loading(20, 0.0, 1.0))));
    run_action(&mut vdom, Action::Toggle, 5).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Paused));

    // Ended → replay.
    vdom.in_runtime(move || np.set(Some(ended_np(20))));
    run_action(&mut vdom, Action::Toggle, 20).await;
    assert_eq!(np_state(&mut vdom), Some(PlaybackState::Loading));
}

/// `resume()` acts only from `Paused` (the low-level primitive behind `play()`):
/// a paused player plays; a `Preparing` player is left alone.
#[tokio::test]
async fn resume_only_acts_from_paused() {
    let mut vdom = mount().await;
    let mut np = now_playing_sig();

    vdom.in_runtime(move || np.set(Some(paused_np(7))));
    vdom.in_runtime(|| controller().resume());
    pump(&mut vdom, 3).await;
    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Playing),
        "a paused player resumes to Playing"
    );

    vdom.in_runtime(move || np.set(Some(NowPlaying::preparing(7))));
    vdom.in_runtime(|| controller().resume());
    pump(&mut vdom, 3).await;
    assert_eq!(
        np_state(&mut vdom),
        Some(PlaybackState::Preparing),
        "resume() must not disturb Preparing"
    );
}

/// `persist_cursor` must skip `Ended`: `on_ended` already wrote the terminal
/// cursor via `MarkPlayed` (reset to 0 + Finished). Persisting the ~duration end
/// position on `stop()`/tab-close would undo that, so a replay resumes at the very
/// end and instantly re-finishes.
#[tokio::test]
async fn persist_cursor_skips_ended() {
    let mut vdom = mount().await;
    let mut np = now_playing_sig();
    vdom.in_runtime(move || {
        np.set(Some(NowPlaying {
            episode_id: 7,
            state: PlaybackState::Ended,
            position_secs: 99.0,
            duration_secs: Some(100.0),
            rate: 1.0,
            buffering: false,
        }))
    });

    vdom.in_runtime(|| controller().persist_cursor());
    pump(&mut vdom, 3).await;

    assert!(
        !commands()
            .iter()
            .any(|c| matches!(c, Command::SetCursor { .. })),
        "persist_cursor must not write an Ended episode's end position"
    );
}
