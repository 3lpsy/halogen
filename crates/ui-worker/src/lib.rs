//! The Halogen sync Web Worker (wasm-bindgen `cdylib`).
//!
//! Runs the sync graph (`halogen_ui_svc_sync::SyncService`) on its own thread, off
//! the main UI thread, behind the `BackgroundRuntime` seam. dx (Dioxus 0.7) has no
//! web-worker target (DioxusLabs/dioxus #3275), so this is our own wasm-bindgen build,
//! bootstrapped by `/worker.js` (`crates/ui/pwa/worker.js`) and produced by the
//! justfile `_worker-build` helper.
//!
//! Protocol ([`ToWorker`]/[`FromWorker`], serde_json strings over `postMessage`):
//! - The main thread posts [`ToWorker::Init`] first, carrying the active-user
//!   namespace `segment` (the worker has its OWN globals and can't read the main
//!   thread's `namespace::segment()`). The worker opens its own
//!   [`WebLocalStore`]/[`WebMediaStore`], builds a [`SyncService`] whose events sink
//!   serializes each [`WorkerEvent`] and posts [`FromWorker::Event`], spawns `run`,
//!   then posts [`FromWorker::Ready`].
//! - Subsequent [`ToWorker::Command`]s are forwarded into the running service's
//!   command channel.
//!
//! IndexedDB is origin-scoped, so the worker's own store connections coexist with the
//! main-thread player's `media` handle — no shared memory.
//!
//! Everything is `#[cfg(target_arch = "wasm32")]`-gated so the native build is an
//! empty crate (`cargo check --workspace` stays clean off-wasm).

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use js_sys::global;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use halogen_ui_commands::Command;
use halogen_ui_svc_media::{MediaStore, WebMediaStore};
use halogen_ui_svc_store::{LocalStore, WebLocalStore};
use halogen_ui_svc_sync::{FromWorker, SyncService, ToWorker, WorkerEvent};

/// Serialize + `postMessage` one worker→main message (log + drop on failure).
fn post(scope: &DedicatedWorkerGlobalScope, msg: &FromWorker) {
    match serde_json::to_string(msg) {
        Ok(s) => {
            if let Err(e) = scope.post_message(&JsValue::from_str(&s)) {
                halogen_ui_logging::error!("worker: postMessage failed: {e:?}");
            }
        }
        Err(e) => halogen_ui_logging::error!("worker: serialize FromWorker failed: {e}"),
    }
}

/// Worker entrypoint, called by `/worker.js` after the wasm module initializes.
///
/// Grabs the `DedicatedWorkerGlobalScope`, installs an `onmessage` handler that
/// drives the [`ToWorker`]/[`FromWorker`] protocol, and returns. Malformed messages
/// are logged and ignored.
#[wasm_bindgen]
pub fn worker_main() {
    // Device logging is FORWARDED, not local: we don't call `halogen_ui_logging::init()`
    // here (its `WASMLayer` console layer needs `web_sys::window()`, which is `None` in a
    // Worker — `init()` is the MAIN thread's job, for the console). Instead, the `Init`
    // handler below installs a worker-safe forwarding subscriber
    // (`halogen_ui_logging::init_forwarding`) that ships each captured `LogLine` to the
    // main thread as `FromWorker::Log`; the main thread `ingest`s it into the single
    // shared ring + `halogen.logs` store. So worker logs land in the same
    // `/logs/device` viewer + storage as the UI's, persisted once by the main thread.
    let scope: DedicatedWorkerGlobalScope = global().unchecked_into();

    // The running service's command sender — `None` until `Init` builds the service.
    let commands: Rc<RefCell<Option<UnboundedSender<Command>>>> = Rc::new(RefCell::new(None));

    let scope_msg = scope.clone();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |evt: MessageEvent| {
        let Some(text) = evt.data().as_string() else {
            halogen_ui_logging::warn!("worker: non-string message ignored");
            return;
        };
        let msg: ToWorker = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                halogen_ui_logging::warn!("worker: malformed ToWorker dropped: {e}");
                return;
            }
        };
        match msg {
            ToWorker::Init {
                segment,
                enabled,
                level,
            } => {
                if commands.borrow().is_some() {
                    halogen_ui_logging::warn!("worker: duplicate Init ignored");
                    return;
                }

                // Seed device-log capture at the main thread's current setting, then
                // install the forwarding subscriber ONCE (this Init arm is reached at
                // most once — the duplicate-Init guard above returns early). After this,
                // the worker's `info!`/`warn!`/`error!` build `LogLine`s and ship them to
                // the main thread (`FromWorker::Log`) for the unified ring + store.
                //
                // `init_forwarding` installs a GLOBAL subscriber, so its closure must be
                // `Send + Sync` — it can't capture the `!Send` worker scope (a `JsValue`).
                // So forward each line through a `Send` channel and `postMessage` it from
                // a `spawn_local` pump. Posting directly (not via `post`, whose `error!`
                // on a failed send would re-enter this forwarding layer → unbounded
                // recursion) also keeps a failed send from looping. A dropped line is
                // harmless.
                halogen_ui_logging::set_enabled(enabled);
                halogen_ui_logging::set_level(level);
                let (log_tx, mut log_rx) = unbounded::<halogen_ui_logging::LogLine>();
                halogen_ui_logging::init_forwarding(move |line| {
                    let _ = log_tx.unbounded_send(line);
                });
                let scope_log = scope_msg.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    while let Some(line) = log_rx.next().await {
                        if let Ok(s) = serde_json::to_string(&FromWorker::Log(line)) {
                            let _ = scope_log.post_message(&JsValue::from_str(&s));
                        }
                    }
                });

                halogen_ui_logging::info!("worker: init for namespace segment {segment}");

                // The service's command channel (fed by subsequent `Command`s).
                let (commands_tx, commands_rx) = unbounded::<Command>();
                *commands.borrow_mut() = Some(commands_tx);

                // Events sink: each published `WorkerEvent` is serialized + posted.
                let (events_tx, mut events_rx) = unbounded::<WorkerEvent>();
                let scope_events = scope_msg.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    while let Some(ev) = events_rx.next().await {
                        post(&scope_events, &FromWorker::Event(ev));
                    }
                });

                // The worker opens its OWN store + media connections under `segment`.
                let store: Rc<dyn LocalStore> = Rc::new(WebLocalStore::new(&segment));
                let media: Option<Rc<dyn MediaStore>> = Some(Rc::new(WebMediaStore::new(&segment)));

                // `run` executes ON this worker thread via `spawn_local`.
                let mut service = SyncService::new(store, media, events_tx);
                // Cross-tab Web Locks (outbox drain, device downloads) scope
                // per account segment — same segment, same locks, every tab.
                service.set_lock_scope(&segment);
                wasm_bindgen_futures::spawn_local(service.run(commands_rx));

                post(&scope_msg, &FromWorker::Ready);
            }
            ToWorker::Command(cmd) => match commands.borrow().as_ref() {
                Some(tx) => {
                    let _ = tx.unbounded_send(cmd);
                }
                None => halogen_ui_logging::warn!("worker: command before Init dropped"),
            },
        }
    });

    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // The worker lives for the page's lifetime — leak the callback so it stays armed.
    onmessage.forget();
}
