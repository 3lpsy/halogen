use std::rc::Rc;

use dioxus::prelude::*;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;

use halogen_ui_appstate::{
    ConnectionState, DownloadState, EpisodeState, HistoryState, PlaybackState, PlaylistState,
    PodcastState, SessionState,
};
use halogen_ui_commands::Command;
use halogen_ui_config::{ClientConfig, ClientConfigStore};
use halogen_ui_svc_media::open_media_handle;
use halogen_ui_svc_store::{LocalStore, StoreHandle};
use halogen_ui_svc_sync::WorkerEvent;
use halogen_ui_toast::ToastQueue;
// `ToastLevel` is only used by the native store-unavailable branch below; on web the
// supervised `WebWorkerRuntime` owns its own fault toasts.
#[cfg(not(target_arch = "wasm32"))]
use halogen_ui_toast::ToastLevel;

#[cfg(not(target_arch = "wasm32"))]
use halogen_ui_svc_store::NativeLocalStore;
#[cfg(target_arch = "wasm32")]
use halogen_ui_svc_store::WebLocalStore;
#[cfg(not(target_arch = "wasm32"))]
use halogen_ui_svc_sync::SyncService;
#[cfg(target_arch = "wasm32")]
use halogen_ui_svc_sync::{BackgroundRuntime, WebWorkerRuntime};

/// Spawns the background sync worker and provides its `Coroutine<Command>` handle.
///
/// The `use_coroutine` hook runs **unconditionally** (rules of hooks). The worker
/// starts idle and only does network I/O after a `SetAuth` command, which this
/// provider sends from the current config and whenever it changes.
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
    let accounts = crate::hooks::use_accounts();
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

    // The worker side of the `BackgroundRuntime` seam — the ONLY part that cfg-splits.
    // The public `dispatch: Coroutine<Command>` and the applier above are identical on
    // both targets, so every caller + the config mirrors / auth-expired effect below
    // are untouched.
    //
    // - native: run `SyncService` IN-PROCESS on the local executor (Stage A behavior).
    //   A dedicated OS thread is a FUTURE swap behind this same trait — NOT attempted
    //   here: native is multi-threaded already, so the wasm-single-thread perf goal
    //   (move the device-download byte fetch + IndexedDB writes off the UI thread)
    //   doesn't apply, and the `Send` refactor `SyncService` would need is out of scope.
    // - web: drive a real `web_sys::Worker` (`WebWorkerRuntime`) running the sync graph
    //   off the main thread; the worker opens its OWN origin-scoped IndexedDB stores.
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
                        halogen_ui_logging::error!("Local store unavailable, sync disabled");
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
                // The web worker opens its OWN store + media (origin-scoped IndexedDB).
                // We still hand the main-thread handles to the runtime: they're used
                // ONLY if the worker crashes enough to fall back to in-process sync
                // (`WebWorkerRuntime` then runs `SyncService` here with these handles).
                // The worker has its own globals, so it can't read the main thread's
                // active-user namespace — pass it explicitly in the Init handshake.
                let segment = halogen_ui_platform::namespace::segment();
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

    // Mirror four config preferences into the worker, each through the same
    // `select` (a PartialEq-gated slice of just that field) → `apply` (the matching
    // `commands::set_*`) shape. The PartialEq memo keeps unrelated config edits from
    // re-dispatching.
    //
    // "Go Offline" is mirrored FIRST on purpose: the worker processes commands FIFO,
    // so a persisted-offline session's `SetOffline` lands ahead of `SetAuth`'s
    // `do_pull` (mirrored last), which then short-circuits — no stray launch pull.
    // `set_auth` runs a full `do_pull` + `drain_outbox`, so it must only react to the
    // auth tuple, never every `ClientConfig` change (font size, nav order, …).
    use_mirror_to_worker(
        config,
        dispatch,
        |c| c.manual_offline,
        crate::commands::set_offline,
    );
    use_mirror_to_worker(
        config,
        dispatch,
        |c| c.playback_prefs.add_to_queue_front,
        crate::commands::set_add_to_queue_front,
    );
    use_mirror_to_worker(
        config,
        dispatch,
        |c| c.download_prefs,
        |d, dp| crate::commands::set_download_prefs(d, dp.chunk_size.bytes(), dp.parallelism),
    );
    use_mirror_to_worker(
        config,
        dispatch,
        |c| (c.server_setup, c.server_url.clone(), c.access_token.clone()),
        |d, (server_setup, server_url, access_token)| {
            if server_setup && let (Some(url), Some(token)) = (server_url, access_token) {
                crate::commands::set_auth(d, url, token);
            }
        },
    );

    // When the worker reports a dead token (401), clear it and stop the worker
    // retrying. Dropping the active config's token makes `RootGuard` redirect to
    // login. The active user stays the active user (registry untouched), so the
    // store namespace is unchanged and a re-login under the same account just
    // refreshes the token in place.
    // Read the dedicated `SessionState` signal: `auth_expired` is its own slice now,
    // so this effect wakes only when the flag actually flips — never on unrelated
    // `EpisodeState` publishes (cursor saves, list updates).
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
                // Embedded accounts never bounce to a password prompt (there is no
                // password the user knows — the credentials live on disk): re-auth
                // silently. `SignOut` goes first — it resets the worker's
                // `auth_expired` flag and drops the dead session while KEEPING the
                // local cache; the fresh token written below then re-fires the
                // auth-tuple mirror above, whose `SetAuth` lands after it (FIFO).
                // On a failed re-auth fall through to the signed-out state (the
                // guard routes embedded accounts to the reconnect page, not login).
                if config.peek().server_kind.is_embedded()
                    && crate::embedded::available()
                    && embedded_relogin_attempts.peek().lt(&3)
                {
                    embedded_relogin_attempts += 1;
                    crate::commands::sign_out(&dispatch);
                    spawn(async move {
                        let fresh = crate::embedded_session::silent_relogin(accounts).await;
                        match fresh {
                            Ok(token) => config.write().access_token = Some(token),
                            Err(e) => {
                                halogen_ui_logging::error!("Embedded re-auth failed: {e}");
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
                crate::commands::sign_out(&dispatch);
            }
        }
    });

    rsx! { {children} }
}

/// Mirror one PartialEq-gated slice of `config` into the worker, re-applying it on
/// boot and whenever just that slice changes. `select` extracts the field from the
/// config; `apply` issues the matching `commands::set_*`. The memo gates on the
/// slice's `PartialEq` so unrelated config edits don't re-dispatch.
///
/// This is a hook (it calls `use_memo`/`use_effect`), so it must be called
/// unconditionally and in a stable order — like the four call sites in
/// `WorkerProvider`.
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
    let segment = halogen_ui_platform::namespace::segment();
    #[cfg(not(target_arch = "wasm32"))]
    let opened: anyhow::Result<Rc<dyn LocalStore>> =
        open_native_store(&segment).map(|s| Rc::new(s) as Rc<dyn LocalStore>);
    #[cfg(target_arch = "wasm32")]
    let opened: anyhow::Result<Rc<dyn LocalStore>> =
        Ok(Rc::new(WebLocalStore::new(&segment)) as Rc<dyn LocalStore>);

    match opened {
        Ok(store) => StoreHandle(Some(store)),
        Err(e) => {
            halogen_ui_logging::error!("Local store unavailable, sync disabled: {e}");
            StoreHandle(None)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_native_store(segment: &str) -> anyhow::Result<NativeLocalStore> {
    // Platform-resolved app data root (XDG / Library / Android files dir —
    // never the CWD). Each user's cache lives in its own subdir so accounts
    // never share rows.
    let dir = halogen_ui_platform::paths::data_root();
    NativeLocalStore::open(dir.join(segment).join("halogen.db"))
}
