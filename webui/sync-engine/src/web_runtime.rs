//! Supervise the dedicated wasm worker with an init/ready handshake, timeout, fault handlers, bounded backoff respawns,
//! then in-process fallback. Restore the account namespace and sticky auth/offline/preferences, but never replay
//! arbitrary sent mutations; recover durable work from IndexedDB. Forward events/logs and notify on faults.

use std::cell::RefCell;
use std::mem::discriminant;
use std::rc::Rc;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{ErrorEvent, MessageEvent, Worker};

use halogen_webui_component_toast::ToastLevel;
use halogen_webui_media::MediaStore;
use halogen_webui_store::LocalStore;

use halogen_webui_commands::Command;

use crate::SyncService;
use crate::runtime::{BackgroundRuntime, FromWorker, ToWorker, WorkerEvent};

/// How many times to respawn a crashed worker before falling back to in-process sync.
const MAX_RESPAWNS: u32 = 3;
/// If `Ready` doesn't arrive within this window after `Init`, treat init as failed.
const READY_TIMEOUT_MS: u32 = 10_000;
/// How long a worker must stay up AFTER `Ready` before the respawn budget clears. `Ready` alone proves nothing, the
/// worker posts it before hydrate / the first pull, so a deterministic post-Ready trap used to clear the budget every
/// cycle and crash-loop forever (toast per ~1s) instead of ever reaching the in-process fallback.
const STABLE_AFTER_MS: u32 = 30_000;

/// Backoff before the n-th respawn (1-indexed): 1s, 2s, 4s, capped at 8s.
fn backoff_ms(attempt: u32) -> u32 {
    1_000u32.saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1).min(3)))
}

/// Which side currently owns the sync loop.
enum Mode {
    /// A live worker (`ready` flips on `FromWorker::Ready`).
    Worker { worker: Worker, ready: bool },
    /// Between a fault and the next worker coming up (faults here are ignored).
    Respawning,
    /// Worker gave up; `SyncService` runs in-process, fed through this channel.
    Fallback(UnboundedSender<Command>),
    /// No local store → can't run sync anywhere (terminal).
    Dead,
}

/// Main-thread supervisor for the sync worker. Single-threaded on wasm, so
/// `Rc<RefCell<_>>` is enough; the JS callbacks hold a `Weak` to avoid a cycle.
struct Supervisor {
    segment: String,
    store: Option<Rc<dyn LocalStore>>,
    media: Option<Rc<dyn MediaStore>>,
    events_tx: UnboundedSender<WorkerEvent>,
    mode: Mode,
    /// Commands buffered until the (current) worker is `Ready`.
    queue: Vec<Command>,
    /// Latest value of each *sticky* command (auth/offline/prefs), replayed to every
    /// fresh worker + the fallback so a restart doesn't lose the session. An ordered
    /// `Vec` deduped by variant — first-seen order is preserved (so `SetOffline` keeps
    /// landing before `SetAuth`, mirroring the provider's mirror order).
    sticky: Vec<Command>,
    respawns: u32,
    /// Bumped on each spawn so a stale ready-timeout from a previous worker no-ops.
    generation: u32,
    /// Kept alive for the *current* worker; replaced on respawn.
    _onmessage: Option<Closure<dyn FnMut(MessageEvent)>>,
    _onerror: Option<Closure<dyn FnMut(ErrorEvent)>>,
    _onmessageerror: Option<Closure<dyn FnMut(MessageEvent)>>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if let Mode::Worker { worker, .. } = &self.mode {
            worker.terminate();
        }
    }
}

/// Commands whose effect must survive a worker restart (mirror the
/// `use_mirror_to_worker` set in `WorkerProvider`): auth, manual-offline, and the
/// queue/download preferences. They're idempotent, so replaying the latest of each
/// to a fresh worker safely re-establishes the session.
fn is_sticky(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::SetAuth { .. }
            | Command::SetOffline(_)
            | Command::SetAddToQueueFront(_)
            | Command::SetDownloadPrefs { .. }
    )
}

/// Serialize + `postMessage` one main→worker message (log + drop on failure).
fn post(worker: &Worker, msg: &ToWorker) {
    match serde_json::to_string(msg) {
        Ok(s) => {
            if let Err(e) = worker.post_message(&JsValue::from_str(&s)) {
                halogen_webui_logging::error!("worker: postMessage failed: {e:?}");
            }
        }
        Err(e) => halogen_webui_logging::error!("worker: serialize ToWorker failed: {e}"),
    }
}

/// Raise a toast on the main thread via the applier's event channel.
fn toast(events_tx: &UnboundedSender<WorkerEvent>, level: ToastLevel, message: impl Into<String>) {
    let _ = events_tx.unbounded_send(WorkerEvent::Toast {
        level,
        message: message.into(),
        timeout_ms: Some(level.default_timeout_ms()),
    });
}

impl Supervisor {
    /// Route a UI-dispatched command to the current backend, caching sticky ones.
    fn dispatch(&mut self, cmd: Command) {
        let sticky = is_sticky(&cmd);
        if sticky {
            // Replace-in-place keeps first-seen order (offline before auth).
            match self
                .sticky
                .iter_mut()
                .find(|c| discriminant(*c) == discriminant(&cmd))
            {
                Some(slot) => *slot = cmd.clone(),
                None => self.sticky.push(cmd.clone()),
            }
        }
        match &self.mode {
            Mode::Worker {
                worker,
                ready: true,
            } => post(worker, &ToWorker::Command(Box::new(cmd))),
            // Before the worker is ready, sticky commands are ALREADY captured in
            // `self.sticky` and `on_ready` replays them — queuing them too would
            // post each twice (re-minting ws-tickets, re-pulling the playlist). Only
            // non-sticky commands need the pre-ready queue.
            Mode::Worker { ready: false, .. } | Mode::Respawning => {
                if !sticky {
                    self.queue.push(cmd);
                }
            }
            Mode::Fallback(tx) => {
                let _ = tx.unbounded_send(cmd);
            }
            Mode::Dead => {}
        }
    }

    /// `FromWorker::Ready`: replay sticky session commands, then flush the queue.
    /// The respawn budget is NOT cleared here — `Ready` precedes hydrate/pull,
    /// so it isn't evidence of health; the stability timer armed at the Ready
    /// message site clears it once the worker has stayed up [`STABLE_AFTER_MS`].
    fn on_ready(&mut self) {
        let worker = match &self.mode {
            Mode::Worker { worker, .. } => worker.clone(),
            _ => return,
        };
        for cmd in &self.sticky {
            post(&worker, &ToWorker::Command(Box::new(cmd.clone())));
        }
        for cmd in std::mem::take(&mut self.queue) {
            post(&worker, &ToWorker::Command(Box::new(cmd)));
        }
        self.mode = Mode::Worker {
            worker,
            ready: true,
        };
    }
}

/// Spawn a fresh worker, wire its callbacks, post `Init`, and arm the ready-timeout.
fn spawn_worker(sup: &Rc<RefCell<Supervisor>>) {
    let worker = match Worker::new("/worker.js") {
        Ok(w) => w,
        Err(e) => {
            halogen_webui_logging::error!("worker: failed to (re)create: {e:?}");
            start_fallback(sup);
            return;
        }
    };

    let generation = {
        let mut s = sup.borrow_mut();
        s.generation = s.generation.wrapping_add(1);
        s.generation
    };

    // onmessage: Ready / Event / Log. Holds a Weak so it can't keep the supervisor
    // (and thus the worker, via Drop) alive past the runtime.
    let weak = Rc::downgrade(sup);
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |evt: MessageEvent| {
        let Some(sup) = weak.upgrade() else { return };
        let Some(text) = evt.data().as_string() else {
            halogen_webui_logging::warn!("worker: non-string message ignored");
            return;
        };
        match serde_json::from_str::<FromWorker>(&text) {
            Ok(FromWorker::Ready) => {
                sup.borrow_mut().on_ready();
                // Clear the respawn budget only once THIS worker generation has
                // stayed ready for the stability window (see STABLE_AFTER_MS).
                let weak_stable = weak.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    TimeoutFuture::new(STABLE_AFTER_MS).await;
                    let Some(sup) = weak_stable.upgrade() else {
                        return;
                    };
                    let mut s = sup.borrow_mut();
                    if s.generation == generation
                        && matches!(s.mode, Mode::Worker { ready: true, .. })
                    {
                        s.respawns = 0;
                    }
                });
            }
            Ok(FromWorker::Event(ev)) => {
                let _ = sup.borrow().events_tx.unbounded_send(ev);
            }
            // A device-log line captured in the worker: ingest it into the MAIN
            // thread's ring + `halogen.logs` store (the single, shared sink).
            Ok(FromWorker::Log(line)) => halogen_webui_logging::ingest(line),
            Err(e) => halogen_webui_logging::warn!("worker: malformed FromWorker dropped: {e}"),
        }
    });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

    let weak_err = Rc::downgrade(sup);
    let onerror = Closure::<dyn FnMut(ErrorEvent)>::new(move |evt: ErrorEvent| {
        if let Some(sup) = weak_err.upgrade() {
            on_fault(
                &sup,
                format!(
                    "uncaught error: {} ({}:{})",
                    evt.message(),
                    evt.filename(),
                    evt.lineno()
                ),
            );
        }
    });
    worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));

    let weak_me = Rc::downgrade(sup);
    let onmessageerror = Closure::<dyn FnMut(MessageEvent)>::new(move |_evt: MessageEvent| {
        if let Some(sup) = weak_me.upgrade() {
            on_fault(
                &sup,
                "messageerror (deserialization) on the worker port".into(),
            );
        }
    });
    worker.set_onmessageerror(Some(onmessageerror.as_ref().unchecked_ref()));

    // Install the new worker + callbacks, then post Init (seeded with the main
    // thread's current device-log setting so worker capture matches).
    {
        let mut s = sup.borrow_mut();
        post(
            &worker,
            &ToWorker::Init {
                segment: s.segment.clone(),
                enabled: halogen_webui_logging::enabled(),
                level: halogen_webui_logging::level(),
            },
        );
        s.mode = Mode::Worker {
            worker,
            ready: false,
        };
        s._onmessage = Some(onmessage);
        s._onerror = Some(onerror);
        s._onmessageerror = Some(onmessageerror);
    }

    // Ready-timeout: if this generation never went `ready`, fault it.
    let weak_to = Rc::downgrade(sup);
    wasm_bindgen_futures::spawn_local(async move {
        TimeoutFuture::new(READY_TIMEOUT_MS).await;
        let Some(sup) = weak_to.upgrade() else { return };
        let stale = {
            let s = sup.borrow();
            s.generation != generation || !matches!(s.mode, Mode::Worker { ready: false, .. })
        };
        if !stale {
            on_fault(&sup, "init timed out (no Ready within 10s)".into());
        }
    });
}

/// Handle a worker fault: terminate it, then respawn (with backoff) or, once the
/// respawn budget is spent, fall back to in-process sync. Idempotent per fault — once
/// the mode leaves `Worker`, repeated faults (a late `onerror` + the ready-timeout)
/// are ignored.
fn on_fault(sup: &Rc<RefCell<Supervisor>>, reason: String) {
    enum Next {
        Respawn(u32),
        Fallback,
    }
    let next = {
        let mut s = sup.borrow_mut();
        if !matches!(s.mode, Mode::Worker { .. }) {
            return; // already respawning / fell back / dead — ignore the duplicate.
        }
        halogen_webui_logging::error!("sync web worker fault: {reason}");
        if let Mode::Worker { worker, .. } = &s.mode {
            worker.terminate();
        }
        s.mode = Mode::Respawning; // gate further faults until the next worker is up.
        s.respawns = s.respawns.saturating_add(1);
        if s.respawns <= MAX_RESPAWNS {
            Next::Respawn(s.respawns)
        } else {
            Next::Fallback
        }
    };

    match next {
        Next::Respawn(attempt) => {
            let delay = backoff_ms(attempt);
            {
                let s = sup.borrow();
                toast(
                    &s.events_tx,
                    ToastLevel::Warning,
                    "Background sync hit a snag — restarting…",
                );
            }
            let sup = sup.clone();
            wasm_bindgen_futures::spawn_local(async move {
                TimeoutFuture::new(delay).await;
                // A re-render could have dropped the runtime in the meantime — only
                // respawn if we're still the owner and still meant to.
                if matches!(sup.borrow().mode, Mode::Respawning) {
                    spawn_worker(&sup);
                }
            });
        }
        Next::Fallback => {
            {
                let s = sup.borrow();
                toast(
                    &s.events_tx,
                    ToastLevel::Warning,
                    "Background sync is running in compatibility mode. Reload the app to retry.",
                );
            }
            start_fallback(sup);
        }
    }
}

/// Run `SyncService` in-process (the same impl the native build uses), feeding it the
/// queued + sticky commands. Degraded (sync back on the main thread) but never frozen.
fn start_fallback(sup: &Rc<RefCell<Supervisor>>) {
    let mut s = sup.borrow_mut();
    let Some(store) = s.store.clone() else {
        halogen_webui_logging::error!("sync web worker: no local store — sync disabled");
        toast(
            &s.events_tx,
            ToastLevel::Error,
            "Local storage is unavailable — changes won't be saved on this device.",
        );
        s.mode = Mode::Dead;
        return;
    };
    halogen_webui_logging::warn!("sync running in-process (web worker fallback)");

    let (cmd_tx, cmd_rx) = unbounded::<Command>();
    // Sticky session commands first (offline→auth order), then anything queued.
    for cmd in &s.sticky {
        let _ = cmd_tx.unbounded_send(cmd.clone());
    }
    for cmd in std::mem::take(&mut s.queue) {
        let _ = cmd_tx.unbounded_send(cmd);
    }
    let mut service = SyncService::new(store, s.media.clone(), s.events_tx.clone());
    // Same cross-tab lock scope as the worker it replaces — the fallback still
    // contends with other tabs' workers over the shared outbox + media store.
    service.set_lock_scope(&s.segment);
    wasm_bindgen_futures::spawn_local(service.run(cmd_rx));
    s.mode = Mode::Fallback(cmd_tx);
}

/// Drives a supervised `web_sys::Worker` running the sync graph off the main thread, implementing
/// [`BackgroundRuntime`]. The UI dispatches [`Command`]s through [`commands`](BackgroundRuntime::commands); a local
/// pump routes them to whichever backend is live (worker, or the in-process fallback), queued until `Ready`. Keep this
/// value alive for the worker's lifetime, dropping it drops the supervisor, which terminates the current worker.
pub struct WebWorkerRuntime {
    commands_tx: UnboundedSender<Command>,
    _sup: Rc<RefCell<Supervisor>>,
}

impl WebWorkerRuntime {
    /// Create + supervise the worker for `segment`, wiring [`FromWorker::Event`]s into
    /// `events_tx`. `store`/`media` are the main-thread handles used only if the worker
    /// fails enough to fall back to in-process sync.
    pub fn new(
        segment: String,
        store: Option<Rc<dyn LocalStore>>,
        media: Option<Rc<dyn MediaStore>>,
        events_tx: UnboundedSender<WorkerEvent>,
    ) -> Self {
        let sup = Rc::new(RefCell::new(Supervisor {
            segment,
            store,
            media,
            events_tx,
            mode: Mode::Respawning, // becomes Worker inside spawn_worker.
            queue: Vec::new(),
            sticky: Vec::new(),
            respawns: 0,
            generation: 0,
            _onmessage: None,
            _onerror: None,
            _onmessageerror: None,
        }));

        spawn_worker(&sup);

        // Local pump: route dispatched commands to the current backend. Holds a strong
        // `sup` ref so the supervisor lives until the command channel closes (app end).
        let (commands_tx, mut commands_rx) = unbounded::<Command>();
        let pump_sup = sup.clone();
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(cmd) = commands_rx.next().await {
                pump_sup.borrow_mut().dispatch(cmd);
            }
        });

        Self {
            commands_tx,
            _sup: sup,
        }
    }
}

impl BackgroundRuntime for WebWorkerRuntime {
    fn commands(&self) -> UnboundedSender<Command> {
        self.commands_tx.clone()
    }
}
