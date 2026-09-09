use dioxus::prelude::*;

#[cfg(any(target_arch = "wasm32", feature = "desktop"))]
use halogen_webui_app_state::PodcastState;
use halogen_webui_app_state::{DownloadState, EpisodeState, PlaybackState, PlaylistState};
use halogen_webui_commands::Command;
use halogen_webui_config::ClientConfig;
use halogen_webui_media::MediaStoreHandle;
use halogen_webui_player::{
    NowPlaying, PlayContext, PlayerController, PlayingIdentity, SleepState,
};
use halogen_webui_player::{PlayerBackend, TICK_INTERVAL_MS};

use halogen_webui_platform::time::sleep_ms;
// Backend per build: wasm → the `<audio>` element (`web`); native with a
// webview renderer → the eval-driven webview backend; renderless native (the
// unit-test graph) → the no-op.
#[cfg(all(not(target_arch = "wasm32"), not(feature = "desktop")))]
use halogen_webui_player::NoopPlayerBackend;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
use halogen_webui_player::WebviewPlayerBackend;
#[cfg(target_arch = "wasm32")]
use halogen_webui_player::web::WebPlayerBackend;

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
    // Memoize playing episode ID and coarse state without position. Rows then avoid recomputing on 250ms position ticks
    // while reacting to actual playback transitions.
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

    // Hardware/OS media controls (Bluetooth, lock screen) → controller. Once; the handlers peek the live config so pref
    // changes apply immediately. Shared between wasm (`media_session`, the browser Media Session API) and the native
    // webview targets (`media_session_webview`, the same API inside the app webview), the two modules expose the same
    // surface. Renderless native builds have neither (no OS media surface to integrate with).
    #[cfg(any(target_arch = "wasm32", feature = "desktop"))]
    {
        use halogen_webui_player::PlaybackState;
        #[cfg(target_arch = "wasm32")]
        use halogen_webui_player_media_session::media_session;
        #[cfg(not(target_arch = "wasm32"))]
        use halogen_webui_player_media_session::media_session_webview as media_session;

        // Retain the OS media-handler guard in Rc for this provider lifetime. Hook cloning shares ownership; final
        // unmount clears browser handlers so per-user remounts cannot leak closures.
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
        // Persist cursor on browser hide/close and refresh/drain on foreground or reconnect. Backgrounding freezes
        // periodic timers, risking lost progress and delayed sync; unmount cleanup prevents duplicate lifecycle
        // listeners after user switches.
        use wasm_bindgen::JsCast;

        use halogen_webui_hook_window_event::use_window_event;
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
                    halogen_webui_commands::actions::refresh(&dispatch);
                }
            },
        );

        let pagehide_controller = controller.clone();
        use_window_event(window.clone().unchecked_into(), "pagehide", move || {
            pagehide_controller.persist_cursor();
        });

        // Reconnected: flush the outbox + pull without waiting for the 60s tick.
        use_window_event(window.unchecked_into(), "online", move || {
            halogen_webui_commands::actions::refresh(&dispatch);
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

    // Watch the selected episode's device-download state and delegate Preparing-to-play/error transitions to the
    // controller. The worker owns downloading; leaving Preparing ends this watcher path without a loop.
    let watch_controller = controller.clone();
    // Memoize episode ID, device state, and integer progress. Progress must participate even while state stays
    // Downloading so advancing downloads reset the Preparing stall guard instead of timing out after five minutes.
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
