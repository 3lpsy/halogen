use std::rc::Rc;

use dioxus::prelude::*;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;

use halogen_webui_app_state::{
    ConnectionState, DownloadState, EpisodeState, HistoryState, PlaybackState, PlaylistState,
    PodcastState, SessionState,
};
use halogen_webui_commands::Command;
use halogen_webui_component_toast::ToastQueue;
use halogen_webui_config::{ClientConfig, ClientConfigStore};
use halogen_webui_media::open_media_handle;
use halogen_webui_store::{LocalStore, StoreHandle};
use halogen_webui_sync_engine::WorkerEvent;
// `ToastLevel` is only used by the native store-unavailable branch below; on web the
// supervised `WebWorkerRuntime` owns its own fault toasts.
#[cfg(not(target_arch = "wasm32"))]
use halogen_webui_component_toast::ToastLevel;

#[cfg(not(target_arch = "wasm32"))]
use halogen_webui_store::NativeLocalStore;
#[cfg(target_arch = "wasm32")]
use halogen_webui_store::WebLocalStore;
#[cfg(not(target_arch = "wasm32"))]
use halogen_webui_sync_engine::SyncService;
#[cfg(target_arch = "wasm32")]
use halogen_webui_sync_engine::{BackgroundRuntime, WebWorkerRuntime};

/// Spawns the background sync worker and provides its `Coroutine<Command>` handle. The `use_coroutine` hook runs
/// **unconditionally** (rules of hooks). The worker starts idle and only does network I/O after a `SetAuth` command,
/// which this provider sends from the current config and whenever it changes.
#[component]
pub fn WorkerProvider(children: Element) -> Element {
    let mut config = use_context::<Signal<ClientConfig>>();
    let app_state = use_context::<Signal<EpisodeState>>();
    let podcasts = use_context::<Signal<PodcastState>>();
    let playlists = use_context::<Signal<PlaylistState>>();
    let playbacks = use_context::<Signal<PlaybackState>>();
    let history = use_context::<Signal<HistoryState>>();
    let downloads = use_context::<Signal<DownloadState>>();
    let connection = use_context::<Signal<ConnectionState>>();
    let session = use_context::<Signal<SessionState>>();
    let accounts = halogen_webui_hook_context::use_accounts();
    let toasts = use_context::<Signal<ToastQueue>>();

    // Open the local store once and share it via context so both the worker (the
    // sole writer) and components (server-paged readers, e.g. `/latest`) use the
    // same backing store.
    let store = use_context_provider(open_store_handle);
    // Audio byte storage (device downloads): worker tasks write it, the player
    // reads it (`audio_url`).
    let media = use_context_provider(open_media_handle);

    // Applier coroutine: owns the main-thread signal + toast-queue writes. The worker
    // is renderer-agnostic — it emits `WorkerEvent`s through a channel instead of
    // touching Dioxus signals — and this drains them here, on the main thread. This is
    // the seam Stage B replaces with a Web Worker `postMessage` bridge.
    let mut app_state = app_state;
    let mut podcasts = podcasts;
    let mut playlists = playlists;
    let mut playbacks = playbacks;
    let mut history = history;
    let mut downloads = downloads;
    let mut connection = connection;
    let mut session = session;
    let mut toasts = toasts;
    let applier = use_coroutine(move |mut rx: UnboundedReceiver<WorkerEvent>| async move {
        while let Some(ev) = rx.next().await {
            match ev {
                WorkerEvent::Episode(s) => app_state.set(s),
                WorkerEvent::Podcasts(s) => podcasts.set(s),
                WorkerEvent::Playlists(s) => playlists.set(s),
                WorkerEvent::Playbacks(s) => playbacks.set(s),
                WorkerEvent::History(s) => history.set(s),
                WorkerEvent::Downloads(s) => downloads.set(s),
                WorkerEvent::Connection(s) => connection.set(s),
                WorkerEvent::Session(s) => session.set(s),
                WorkerEvent::Toast {
                    level,
                    message,
                    timeout_ms,
                } => {
                    toasts.write().push(level, message, timeout_ms);
                }
            }
        }
    });
    let events_tx = applier.tx();

    // Keep dispatch and event application identical across targets. Native runs SyncService on the local executor; web
    // uses a dedicated worker with its own IndexedDB connections. Only this runtime selection is target-specific.
    let worker_store = store.clone();
    let worker_media = media.clone();
    let dispatch = use_coroutine(move |rx: UnboundedReceiver<Command>| {
        let worker_store = worker_store.clone();
        let worker_media = worker_media.clone();
        let events_tx = events_tx.clone();
        async move {
            #[cfg(not(target_arch = "wasm32"))]
            {
                match worker_store.0 {
                    Some(store) => {
                        SyncService::new(store, worker_media.0, events_tx.clone())
                            .run(rx)
                            .await;
                    }
                    None => {
                        // The store failed to open (native disk/permission error; the
                        // web store never fails). Without it nothing can be persisted,
                        // so tell the user once rather than silently dropping every
                        // action they take.
                        halogen_webui_logging::error!("Local store unavailable, sync disabled");
                        let _ = events_tx.unbounded_send(WorkerEvent::Toast {
                            level: ToastLevel::Error,
                            message: "Local storage is unavailable — changes won't be saved on \
                                      this device."
                                .into(),
                            timeout_ms: Some(ToastLevel::Error.default_timeout_ms()),
                        });
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                // Pass the account namespace explicitly because worker globals are independent. Retain main-thread
                // store/media handles only for in-process fallback after repeated worker crashes.
                let segment = halogen_webui_platform::namespace::segment();
                // The runtime supervises the worker (respawn on crash, then fall back
                // to in-process sync) and never fails to construct — a worker that
                // can't even be created falls straight back. Holding `runtime` for the
                // coroutine's lifetime keeps the worker (or the fallback) alive.
                let runtime =
                    WebWorkerRuntime::new(segment, worker_store.0, worker_media.0, events_tx);
                let cmd_tx = runtime.commands();
                let mut rx = rx;
                while let Some(cmd) = rx.next().await {
                    let _ = cmd_tx.unbounded_send(cmd);
                }
                drop(runtime);
            }
        }
    });

    use_context_provider(|| dispatch);

    // Mirror equality-gated config slices, sending offline state before auth so persisted offline mode suppresses
    // startup pulling. Auth must react only to its tuple; unrelated preference changes must not trigger pull/drain.
    use_mirror_to_worker(
        config,
        dispatch,
        |c| c.manual_offline,
        halogen_webui_commands::actions::set_offline,
    );
    use_mirror_to_worker(
        config,
        dispatch,
        |c| c.playback_prefs.add_to_queue_front,
        halogen_webui_commands::actions::set_add_to_queue_front,
    );
    use_mirror_to_worker(
        config,
        dispatch,
        |c| c.download_prefs,
        |d, dp| {
            halogen_webui_commands::actions::set_download_prefs(
                d,
                dp.chunk_size.bytes(),
                dp.parallelism,
            )
        },
    );
    use_mirror_to_worker(
        config,
        dispatch,
        |c| (c.server_setup, c.server_url.clone(), c.access_token.clone()),
        |d, (server_setup, server_url, access_token)| {
            if server_setup && let (Some(url), Some(token)) = (server_url, access_token) {
                halogen_webui_commands::actions::set_auth(d, url, token);
            }
        },
    );

    // On rejected auth, clear the token and stop retries while retaining the active account namespace for re-login.
    // Subscribe only to SessionState so unrelated data publications do not rerun expiration handling.
    let auth_expired = use_memo(move || session.read().auth_expired);
    // Embedded silent re-auth is bounded: a server that mints tokens the
    // worker then rejects (corrupted secret, clock skew) must degrade to the
    // ordinary signed-out state after a few tries, not loop boot+login
    // forever. Reset on remount (a switch/reconnect starts fresh).
    let mut embedded_relogin_attempts = use_signal(|| 0u8);
    use_effect(move || {
        if auth_expired() {
            // Only act once: `auth_expired` stays true across publishes until the
            // store re-mounts, so clearing the token AND telling the worker to sign
            // out must both be gated on there still being a token to clear —
            // otherwise every subsequent state publish re-dispatches `SignOut`.
            if config.peek().access_token.is_some() {
                // Reauthenticate local accounts silently from disk credentials. Send SignOut first to clear expired
                // state while retaining cache, then fresh config triggers FIFO SetAuth. Failure routes to local
                // reconnect rather than a password prompt.
                if config.peek().server_kind.is_embedded()
                    && halogen_webui_provider_local::embedded::available()
                    && embedded_relogin_attempts.peek().lt(&3)
                {
                    embedded_relogin_attempts += 1;
                    halogen_webui_commands::actions::sign_out(&dispatch);
                    spawn(async move {
                        let fresh = halogen_webui_provider_local::embedded_session::silent_relogin(
                            accounts,
                        )
                        .await;
                        match fresh {
                            Ok(token) => config.write().access_token = Some(token),
                            Err(e) => {
                                halogen_webui_logging::error!("Embedded re-auth failed: {e}");
                                config.write().access_token = None;
                            }
                        }
                        let saved = config.peek().clone();
                        ClientConfigStore::save(&saved).await;
                    });
                    return;
                }
                // Single-field read-modify-write through `.write()` (which notifies
                // subscribers like `.set()`) so a concurrent config edit to another
                // field isn't clobbered by a stale full-struct snapshot.
                config.write().access_token = None;
                let saved = config.peek().clone();
                spawn(async move { ClientConfigStore::save(&saved).await });
                halogen_webui_commands::actions::sign_out(&dispatch);
            }
        }
    });

    rsx! { {children} }
}

/// Mirror one PartialEq config slice on boot and change, avoiding unrelated redispatches. This calls hooks, so every
/// call must be unconditional and ordered consistently.
fn use_mirror_to_worker<T: PartialEq + Clone + 'static>(
    config: Signal<ClientConfig>,
    dispatch: Coroutine<Command>,
    select: impl Fn(&ClientConfig) -> T + 'static,
    apply: impl Fn(&Coroutine<Command>, T) + 'static,
) {
    let slice = use_memo(move || select(&config.read()));
    use_effect(move || {
        apply(&dispatch, slice());
    });
}

/// Open the platform local store and wrap it as a shared `StoreHandle`. Runs once
/// (memoised by `use_context_provider`). A failure to open disables sync/caching
/// rather than crashing the app.
fn open_store_handle() -> StoreHandle {
    // Per-user namespace, set by `AccountsProvider` before this subtree mounts.
    // The keyed remount on a user switch re-runs this with the new segment.
    let segment = halogen_webui_platform::namespace::segment();
    #[cfg(not(target_arch = "wasm32"))]
    let opened: anyhow::Result<Rc<dyn LocalStore>> =
        open_native_store(&segment).map(|s| Rc::new(s) as Rc<dyn LocalStore>);
    #[cfg(target_arch = "wasm32")]
    let opened: anyhow::Result<Rc<dyn LocalStore>> =
        Ok(Rc::new(WebLocalStore::new(&segment)) as Rc<dyn LocalStore>);

    match opened {
        Ok(store) => StoreHandle(Some(store)),
        Err(e) => {
            halogen_webui_logging::error!("Local store unavailable, sync disabled: {e}");
            StoreHandle(None)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_native_store(segment: &str) -> anyhow::Result<NativeLocalStore> {
    // Platform-resolved app data root (XDG / Library / Android files dir —
    // never the CWD). Each user's cache lives in its own subdir so accounts
    // never share rows.
    let dir = halogen_webui_platform::paths::data_root();
    NativeLocalStore::open(dir.join(segment).join("halogen.db"))
}
