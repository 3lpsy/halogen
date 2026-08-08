use dioxus::prelude::*;

#[cfg(any(target_arch = "wasm32", feature = "desktop"))]
use halogen_ui_appstate::PodcastState;
use halogen_ui_appstate::{DownloadState, EpisodeState, PlaybackState, PlaylistState};
use halogen_ui_commands::Command;
use halogen_ui_config::ClientConfig;
use halogen_ui_svc_media::MediaStoreHandle;
use halogen_ui_svc_player::{
    NowPlaying, PlayContext, PlayerController, PlayingIdentity, SleepState,
};
use halogen_ui_svc_player::{PlayerBackend, TICK_INTERVAL_MS};

use halogen_ui_platform::time::sleep_ms;
// Backend per build: wasm → the `<audio>` element (`web`); native with a
// webview renderer → the eval-driven webview backend; renderless native (the
// unit-test graph) → the no-op.
#[cfg(all(not(target_arch = "wasm32"), not(feature = "desktop")))]
use halogen_ui_svc_player::NoopPlayerBackend;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
use halogen_ui_svc_player::WebviewPlayerBackend;
#[cfg(target_arch = "wasm32")]
use halogen_ui_svc_player::web::WebPlayerBackend;

/// Constructs the platform audio backend + `PlayerController`, provides it (and
/// the `now_playing` signal) as context, and runs a poll loop that reflects
/// playback into that signal.
#[component]
pub fn PlayerProvider(children: Element) -> Element {
    let app_state = use_context::<Signal<EpisodeState>>();
    let playlists = use_context::<Signal<PlaylistState>>();
    let playbacks = use_context::<Signal<PlaybackState>>();
    let downloads = use_context::<Signal<DownloadState>>();
    let dispatch = use_context::<Coroutine<Command>>();
    let config = use_context::<Signal<ClientConfig>>();
    // Provided by WorkerProvider (an ancestor): the device audio byte store.
    let media = use_context::<MediaStoreHandle>();

    // Player state lives in its own signal (NOT EpisodeState) so the controller's
    // ~4×/sec updates only re-render the player UI, not the whole app. Provided
    // as context for the mini + full-screen players to read.
    let now_playing = use_context_provider(|| Signal::new(None::<NowPlaying>));
    // Position-free projection of `now_playing` (episode_id + coarse state), provided
    // as a `PartialEq`-gated `Memo` for the episode rows. The controller writes
    // `now_playing` ~4×/sec for position ticks; the rows only need identity, so a row
    // that reads THIS memo isn't invalidated by a tick (the projection's value is
    // unchanged). Without it, each tick recomputed every visible row's row-state memo
    // (~30 rows × 4/sec). See `use_now_playing_identity` / `resolve_episode_row_state`.
    let now_playing_id = use_memo(move || now_playing.read().as_ref().map(PlayingIdentity::from));
    use_context_provider(|| now_playing_id);
    // Sleep-timer state, provided as context (read by the player's sleep button) and
    // owned/ticked by the controller.
    let sleep = use_context_provider(|| Signal::new(SleepState::default()));
    // Play context (the playlist the user pressed play from), provided as context
    // (read by "up next" surfaces) and written by the controller's `*_in` entries.
    let play_context = use_context_provider(|| Signal::new(PlayContext::default()));

    let controller = use_hook(|| {
        #[cfg(target_arch = "wasm32")]
        let backend: Box<dyn PlayerBackend> = Box::new(WebPlayerBackend::new());
        #[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
        let backend: Box<dyn PlayerBackend> = Box::new(WebviewPlayerBackend::new());
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "desktop")))]
        let backend: Box<dyn PlayerBackend> = Box::new(NoopPlayerBackend);

        PlayerController::new(
            backend,
            app_state,
            playlists,
            playbacks,
            downloads,
            now_playing,
            dispatch,
            media.0,
            config,
            sleep,
            play_context,
        )
    });

    use_context_provider(|| Signal::new(controller.clone()));

    // Hardware/OS media controls (Bluetooth, lock screen) → controller. Once;
    // the handlers peek the live config so pref changes apply immediately.
    // Shared between wasm (`media_session`, the browser Media Session API) and
    // the native webview targets (`media_session_webview`, the same API inside
    // the app webview) — the two modules expose the same surface. Renderless
    // native builds have neither (no OS media surface to integrate with).
    #[cfg(any(target_arch = "wasm32", feature = "desktop"))]
    {
        use halogen_ui_svc_player::PlaybackState;
        #[cfg(target_arch = "wasm32")]
        use halogen_ui_svc_player::media_session;
        #[cfg(not(target_arch = "wasm32"))]
        use halogen_ui_svc_player::media_session_webview as media_session;

        // Register the OS media-control handlers and STORE the returned guard so it
        // lives exactly as long as this provider. PlayerProvider remounts per
        // user-switch; on unmount the scope drops this `Rc`, dropping the guard,
        // which clears the browser handlers (and frees the closures) — so remounts
        // don't leak a fresh closure set each time. `Rc` because `use_hook` clones
        // its value each render and the guard itself isn't `Clone`; cloning the `Rc`
        // just shares ownership (no premature drop).
        let ms_controller = controller.clone();
        use_hook(move || std::rc::Rc::new(media_session::register(ms_controller, config)));

        // Lock-screen/car "now playing" metadata. Memoized on the episode id so
        // the controller's ~4×/sec position writes don't rebuild MediaMetadata;
        // also subscribed to app_state because title/artwork can arrive after
        // playback starts (lazy episode fetch).
        let current_episode = use_memo(move || now_playing.read().as_ref().map(|n| n.episode_id));
        // Podcast titles live in their own slice now; read it here (inside the
        // wasm-only block) so the metadata memo can resolve the show name.
        let podcasts = use_context::<Signal<PodcastState>>();
        // Memoize the metadata tuple so the JS `update_metadata` (and the
        // MediaMetadata rebuild) only fires when title/podcast/art actually change —
        // the memo absorbs the per-publish churn, its PartialEq output gates the effect.
        let metadata = use_memo(move || {
            current_episode().map(|id| {
                app_state
                    .read()
                    .episode_display(&podcasts.read(), id, config.read().server_url.as_deref())
                    .unwrap_or_else(|| ("Now Playing".to_string(), String::new(), None, None))
            })
        });
        use_effect(move || match metadata() {
            // OS lock-screen art wants the full image, not the list thumbnail.
            Some((title, podcast, art, _art_small)) => {
                media_session::update_metadata(&title, &podcast, art.as_deref())
            }
            None => media_session::clear_metadata(),
        });

        // Play/pause state for the OS controls (they can't observe the element).
        let is_playing = use_memo(move || {
            now_playing
                .read()
                .as_ref()
                .map(|n| n.state == PlaybackState::Playing)
        });
        use_effect(move || {
            if let Some(playing) = is_playing() {
                media_session::update_playback_state(playing);
            }
        });

        // Progress bar on the lock screen: refreshed each poll tick (cheap; also
        // covers seeks and rate changes).
        use_effect(move || {
            if let Some(np) = now_playing.read().as_ref() {
                media_session::update_position_state(np.position_secs, np.duration_secs, np.rate);
            }
        });
    }

    // Browser lifecycle bridge — genuinely wasm-only: a native window has no
    // tab freeze/`pagehide`, and the OS never suspends the webview under the
    // running process the way mobile browsers background a tab.
    #[cfg(target_arch = "wasm32")]
    {
        // Browser background / close / reconnect lifecycle. The JS poll tick that
        // drives cursor persistence (a ~10s debounce, see `controller::tick`) and
        // the worker's 60s sync pull both FREEZE when a mobile browser backgrounds
        // the tab — so a close mid-play would lose up to ~10s of progress, and a
        // background→return would wait up to 60s before refreshing + draining the
        // outbox. Bridge both with DOM lifecycle events: persist on hide/close,
        // pull+drain on foreground/reconnect. `use_window_event` tears the listeners
        // down on unmount, so the per-user-switch remount of this provider can't
        // stack duplicate handlers.
        use wasm_bindgen::JsCast;

        use crate::hooks::use_window_event;
        let window = web_sys::window().expect("window");
        let document = window.document().expect("document");

        let hide_controller = controller.clone();
        use_window_event(
            document.clone().unchecked_into(),
            "visibilitychange",
            move || {
                let hidden = web_sys::window()
                    .and_then(|w| w.document())
                    .is_some_and(|d| d.hidden());
                if hidden {
                    // Save now — the frozen tick will never reach the ~10s debounce.
                    hide_controller.persist_cursor();
                } else {
                    // Foreground again: pull fresh + drain queued actions at once.
                    crate::commands::refresh(&dispatch);
                }
            },
        );

        let pagehide_controller = controller.clone();
        use_window_event(window.clone().unchecked_into(), "pagehide", move || {
            pagehide_controller.persist_cursor();
        });

        // Reconnected: flush the outbox + pull without waiting for the 60s tick.
        use_window_event(window.unchecked_into(), "online", move || {
            crate::commands::refresh(&dispatch);
        });
    }

    // Poll backend events into the `now_playing` signal ~4×/sec while mounted.
    let tick_controller = controller.clone();
    use_future(move || {
        let controller = tick_controller.clone();
        async move {
            loop {
                sleep_ms(TICK_INTERVAL_MS as u32).await;
                controller.tick();
            }
        }
    });

    // Auto-play-when-ready: subscribe to now_playing + the CLIENT download state and
    // hand both to the controller, which owns the `Preparing → play / Error`
    // transition (the only writer of `now_playing`). The worker's spawned download
    // task owns the pipeline (wait for the server copy, fetch the bytes, write the
    // media store) and reports through `client_downloads`, so that's the only signal
    // the player needs. `play_episode` flips state off `Preparing`, so no loop.
    let watch_controller = controller.clone();
    // Narrow to (episode_id, device_state, device progress) via a PartialEq memo
    // so this only fires when the relevant download state actually changes — not
    // on every EpisodeState publish while a download is in flight (the handler is
    // a no-op otherwise). The PROGRESS percent must ride along: the coarse state
    // sits at `Downloading` for the whole transfer, so without it the effect
    // fired exactly once and the controller's 5-minute Preparing stall guard
    // never saw the forward progress that is supposed to reset its window — a
    // slow-but-advancing large download timed out spuriously. Each advancing
    // integer percent re-fires the effect → `note_preparing_progress`.
    let preparing_target = use_memo(move || {
        let episode_id = now_playing.read().as_ref().map(|n| n.episode_id)?;
        let d = downloads.read();
        Some((
            episode_id,
            d.device_state(episode_id),
            d.download_progress.get(&episode_id).copied(),
        ))
    });
    use_effect(move || {
        if let Some((episode_id, state, _progress)) = preparing_target() {
            watch_controller.handle_preparing_download_state(episode_id, state);
        }
    });

    rsx! { {children} }
}
