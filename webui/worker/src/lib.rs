//! Run wasm SyncService behind the JSON ToWorker/FromWorker protocol. Init supplies the namespace, opens worker-owned
//! IndexedDB handles, starts the service, and replies Ready; later commands forward to it and events return to the main
//! thread. Native compiles an empty crate.

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

use halogen_webui_commands::Command;
use halogen_webui_media::{MediaStore, WebMediaStore};
use halogen_webui_store::{LocalStore, WebLocalStore};
use halogen_webui_sync_engine::{FromWorker, SyncService, ToWorker, WorkerEvent};

/// Serialize + `postMessage` one worker→main message (log + drop on failure).
fn post(scope: &DedicatedWorkerGlobalScope, msg: &FromWorker) {
    match serde_json::to_string(msg) {
        Ok(s) => {
            if let Err(e) = scope.post_message(&JsValue::from_str(&s)) {
                halogen_webui_logging::error!("worker: postMessage failed: {e:?}");
            }
        }
        Err(e) => halogen_webui_logging::error!("worker: serialize FromWorker failed: {e}"),
    }
}

/// Worker entrypoint, called by `/worker.js` after the wasm module initializes. Grabs the `DedicatedWorkerGlobalScope`,
/// installs an `onmessage` handler that drives the [`ToWorker`]/[`FromWorker`] protocol, and returns. Malformed
/// messages are logged and ignored.
#[wasm_bindgen]
pub fn worker_main() {
    // Forward worker LogLines to the main thread's shared ring/store instead of initializing window-dependent console
    // logging. init_forwarding works without web_sys::window and avoids duplicate persistence.
    let scope: DedicatedWorkerGlobalScope = global().unchecked_into();

    // The running service's command sender — `None` until `Init` builds the service.
    let commands: Rc<RefCell<Option<UnboundedSender<Command>>>> = Rc::new(RefCell::new(None));

    let scope_msg = scope.clone();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |evt: MessageEvent| {
        let Some(text) = evt.data().as_string() else {
            halogen_webui_logging::warn!("worker: non-string message ignored");
            return;
        };
        let msg: ToWorker = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                halogen_webui_logging::warn!("worker: malformed ToWorker dropped: {e}");
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
                    halogen_webui_logging::warn!("worker: duplicate Init ignored");
                    return;
                }

                // Install forwarding once after seeding log preferences. Its Send+Sync subscriber writes a Send
                // channel; a local pump posts messages without capturing the non-Send worker scope. Do not use the
                // logging post helper, whose send-error log would recursively re-enter this subscriber.
                halogen_webui_logging::set_enabled(enabled);
                halogen_webui_logging::set_level(level);
                let (log_tx, mut log_rx) = unbounded::<halogen_webui_logging::LogLine>();
                halogen_webui_logging::init_forwarding(move |line| {
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

                halogen_webui_logging::info!("worker: init for namespace segment {segment}");

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
                    let _ = tx.unbounded_send(*cmd);
                }
                None => halogen_webui_logging::warn!("worker: command before Init dropped"),
            },
        }
    });

    scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    // The worker lives for the page's lifetime — leak the callback so it stays armed.
    onmessage.forget();
}
