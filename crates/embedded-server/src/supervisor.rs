//! The process-global supervisor: one manager task that starts, restarts, and
//! stops the in-process server on command.
//!
//! Lifecycle:
//!
//! ```text
//! ensure_started ──poke──▶ manager: acquire lock ─▶ ┌─ run: init ─ serve ─┐
//!        (binds the port synchronously,             │   ▲                │
//!         so the base URL is known before           │   │ restart        ▼
//!         any async init has run)                   │   └── RestartHandle / stop()
//!                                                   └─ Stopped / Failed ─┘
//! ```
//!
//! - The **port is sticky**: chosen once (OS-assigned), rebound with
//!   `SO_REUSEADDR` across in-process restarts (a graceful drain leaves the
//!   old accepted sockets in TIME_WAIT on it). Only if that rebind loses the
//!   port does the supervisor fall back to a fresh ephemeral one — hosts can
//!   watch [`subscribe_status`] to re-publish the base URL.
//! - **stop() is generational**: a run captures the stop generation at start
//!   and drains when it moves past — no lost-wakeup window, idempotent.
//! - The **single-instance lock** (`<root>/lock`, `File::try_lock`) is
//!   acquired once and held for the process lifetime (across restarts);
//!   [`destroy`] releases it. Two app processes on one library would mean two
//!   polling loops + two download watchdogs racing the same files.

use std::fs;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use halogen_migrate::connect_and_migrate_wal;
use halogen_orm::user;
use halogen_polling::PollingHandle;
use halogen_server::restart::RestartHandle;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use tokio::net::TcpSocket;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use crate::config::{build_config, log_config_overrides};
use crate::dirs::EmbeddedDirs;
use crate::secrets::{Credentials, Secrets};

/// Where the embedded server is in its lifecycle. `Starting`/`Running` carry
/// the bound port (known from the moment a start is requested).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedStatus {
    Stopped,
    Starting { port: u16 },
    Running { port: u16 },
    Failed { error: String },
}

struct Shared {
    dirs: EmbeddedDirs,
    /// The supervisor's OWN runtime, on its own threads. Boot must not depend
    /// on the host's ambient tokio context (dioxus contexts vary across
    /// platforms and the `dx serve` dev harness), and server load stays off
    /// the UI scheduler entirely. Held here so it lives for the process.
    runtime: tokio::runtime::Runtime,
    status_tx: watch::Sender<EmbeddedStatus>,
    /// Start pokes, stamped with the [`Self::stop_gen`] current when they were
    /// SENT. The manager blocks inside a run for its whole lifetime, so pokes
    /// issued while running queue up — the stamp lets it discard the ones that
    /// predate the latest `stop()`/`destroy()`, which would otherwise
    /// resurrect a deliberately-stopped server the moment the run drains.
    start_tx: mpsc::UnboundedSender<u64>,
    /// Monotonic stop generation: a run started at generation G drains when
    /// the generation moves past G.
    stop_gen: watch::Sender<u64>,
    /// Last bound port — the rebind target across restarts.
    port: Mutex<Option<u16>>,
    /// Socket staged for the next run: BOUND (so `ensure_started` knows the
    /// port synchronously, from any thread — bind is a plain syscall) but not
    /// yet listening — `listen()` registers with a reactor, so the manager
    /// does it on the supervisor runtime as the run starts.
    pending_socket: Mutex<Option<TcpSocket>>,
    /// Held single-instance lock (`<root>/lock`). `Some` from first
    /// successful start until `destroy`.
    lock_file: Mutex<Option<fs::File>>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

enum RunEnd {
    Restart,
    Stop,
    Fatal(String),
}

/// Idempotent. On the first call: binds the loopback port **synchronously**
/// (the base URL is returnable before any async init), spawns the manager on
/// the supervisor's own runtime, and requests a start. Later calls just
/// (re)request a start — a no-op while Starting/Running. Returns the base URL.
pub fn ensure_started(dirs: EmbeddedDirs) -> Result<String, String> {
    let shared = shared_or_init(dirs)?;
    let port = shared.staged_port()?;
    // The manager coalesces pokes and ignores them while a run is live. The
    // stop-generation stamp marks this request as issued BEFORE any later
    // stop(): if one lands first, the queued poke is dead on arrival instead
    // of rebooting the stopped server.
    let _ = shared.start_tx.send(*shared.stop_gen.borrow());
    info!("Embedded server start requested (port {port})");
    Ok(base_url_for(port))
}

/// Resolves once the requested start reaches `Running` (Ok(port)) or
/// `Failed` (Err). Callers surface the error with a Retry (a retry is just
/// another [`ensure_started`] + `wait_ready`).
pub async fn wait_ready() -> Result<u16, String> {
    let Some(shared) = SHARED.get() else {
        return Err("Embedded server was never started".to_string());
    };
    let mut rx = shared.status_tx.subscribe();
    loop {
        let current = rx.borrow_and_update().clone();
        match current {
            EmbeddedStatus::Running { port } => return Ok(port),
            EmbeddedStatus::Failed { error } => return Err(error),
            EmbeddedStatus::Stopped | EmbeddedStatus::Starting { .. } => {}
        }
        if rx.changed().await.is_err() {
            return Err("Embedded server supervisor is gone".to_string());
        }
    }
}

pub fn status() -> EmbeddedStatus {
    match SHARED.get() {
        Some(shared) => shared.status_now(),
        None => EmbeddedStatus::Stopped,
    }
}

/// Watch the lifecycle — hosts use this to re-publish the base URL on the
/// rare port change and to surface Failed states in settings.
pub fn subscribe_status() -> Option<watch::Receiver<EmbeddedStatus>> {
    SHARED.get().map(|shared| shared.status_tx.subscribe())
}

/// The loopback base URL, as soon as a port has ever been chosen this
/// process (regardless of current status — a Failed server's URL simply
/// doesn't answer, which downstream already handles as offline).
pub fn base_url() -> Option<String> {
    let shared = SHARED.get()?;
    let port = (*shared.port.lock().expect("port lock poisoned"))?;
    Some(base_url_for(port))
}

/// Gracefully stop and wait for `Stopped`/`Failed`. Idempotent; a later
/// [`ensure_started`] boots it again (same port when still free).
pub async fn stop() {
    let Some(shared) = SHARED.get() else {
        return;
    };
    shared.stop_gen.send_modify(|g| *g += 1);
    let mut rx = shared.status_tx.subscribe();
    loop {
        let terminal = matches!(
            *rx.borrow_and_update(),
            EmbeddedStatus::Stopped | EmbeddedStatus::Failed { .. }
        );
        if terminal || rx.changed().await.is_err() {
            return;
        }
    }
}

/// The admin credentials for silent login. Errors if never provisioned.
pub fn credentials(dirs: &EmbeddedDirs) -> Result<Credentials, String> {
    match Secrets::load(dirs)? {
        Some(secrets) => Ok(secrets.credentials()),
        None => Err("Embedded server has not been provisioned yet".to_string()),
    }
}

/// The stored silent-login credentials for a SPECIFIC embedded user (the
/// seeded admin or an account added through the embedded add-user flow).
pub fn credentials_for(dirs: &EmbeddedDirs, username: &str) -> Result<Credentials, String> {
    let secrets = Secrets::load(dirs)?
        .ok_or_else(|| "Embedded server has not been provisioned yet".to_string())?;
    let password = secrets
        .password_for(username)
        .ok_or_else(|| format!("No stored credentials for '{username}'"))?
        .to_string();
    Ok(Credentials {
        username: username.to_string(),
        password,
    })
}

/// Fresh credentials for a to-be-created embedded user — NOT persisted.
/// Persist with [`remember_user`] only after the server-side create succeeds,
/// so a failed create (duplicate name, validation) can never clobber an
/// existing user's stored secret.
pub fn generate_credentials(username: &str) -> Credentials {
    Credentials {
        username: username.to_lowercase(),
        password: Secrets::fresh_password(),
    }
}

/// Persist a user's silent-login credentials into `secrets.json` (upsert).
pub fn remember_user(dirs: &EmbeddedDirs, creds: &Credentials) -> Result<(), String> {
    let mut secrets = Secrets::load_or_create(dirs)?;
    secrets.set_password(&creds.username, creds.password.clone());
    secrets.save(dirs)
}

/// Whether `secrets.json` holds credentials for `username` (any kind).
pub fn has_credentials(dirs: &EmbeddedDirs, username: &str) -> Result<bool, String> {
    Ok(Secrets::load(dirs)?.is_some_and(|s| s.password_for(&username.to_lowercase()).is_some()))
}

/// Auth self-heal for a SPECIFIC user: rotate their password directly in the
/// DB (we own the file) and record it in `secrets.json`. Covers silent-login
/// drift for added users and re-keys users an import created with random
/// passwords. Unlike [`recover_admin`] this never seeds and never adopts a
/// renamed admin — the row must exist. Password-only rotation is safe against
/// a RUNNING server (logins verify against the DB per request).
/// Rotate one user row's password hash in place (the shared core of both
/// recovery paths — bcrypt cost, `updated_at` touch, and the update shape live
/// here once). Password-only rotation is safe against a RUNNING server:
/// logins verify against the DB per request.
async fn rotate_password_in_db(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    password: &str,
) -> Result<(), String> {
    let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| format!("Failed to hash recovery password: {e}"))?;
    let update = user::ActiveModel {
        id: Set(user_id),
        password_hash: Set(hash),
        updated_at: Set(Utc::now()),
        ..Default::default()
    };
    update
        .update(dbc)
        .await
        .map_err(|e| format!("Failed to update user {user_id}'s password: {e}"))?;
    Ok(())
}

pub async fn recover_user(dirs: &EmbeddedDirs, username: &str) -> Result<Credentials, String> {
    let uname = username.to_lowercase();
    let db_path = dirs.db_path();
    if !db_path.exists() {
        return Err("Embedded database does not exist yet".to_string());
    }
    let mut secrets = Secrets::load_or_create(dirs)?;
    let password = Secrets::fresh_password();

    let dbc = connect_and_migrate_wal(&db_path, false)
        .await
        .map_err(|e| format!("Failed to open embedded DB for recovery: {e}"))?;
    let row = user::Entity::find()
        .filter(user::Column::Username.eq(uname.clone()))
        .one(&dbc)
        .await
        .map_err(|e| format!("Failed to look up user '{uname}': {e}"))?;
    let outcome = match row {
        Some(u) => rotate_password_in_db(&dbc, u.id, &password).await,
        None => Err(format!("No embedded user '{uname}' to recover")),
    };
    let _ = dbc.close().await;
    outcome?;

    secrets.set_password(&uname, password.clone());
    secrets.save(dirs)?;
    Ok(Credentials {
        username: uname,
        password,
    })
}

/// Auth self-heal: rotate the admin password directly in the DB (we own the
/// file) and rewrite `secrets.json`. Covers a lost/stale secrets file and an
/// out-of-band password change. Call while stopped — or restart after — so a
/// regenerated signing secret (missing-file case) is what the server signs
/// with. No-ops on the DB when it doesn't exist yet (fresh library: the
/// normal seed path applies these credentials at next start).
pub async fn recover_admin(dirs: &EmbeddedDirs) -> Result<Credentials, String> {
    let mut secrets = match Secrets::load(dirs)? {
        Some(existing) => existing,
        None => Secrets::generate(),
    };
    secrets.rotate_password();

    let db_path = dirs.db_path();
    if db_path.exists() {
        let dbc = connect_and_migrate_wal(&db_path, false)
            .await
            .map_err(|e| format!("Failed to open embedded DB for recovery: {e}"))?;
        let admin = user::Entity::find()
            .filter(user::Column::IsAdmin.eq(true))
            .order_by_asc(user::Column::Id)
            .one(&dbc)
            .await
            .map_err(|e| format!("Failed to look up admin user: {e}"))?;
        match admin {
            Some(admin) => {
                // Track a renamed admin too — silent login must target the
                // row that actually exists.
                secrets.admin_username = admin.username.clone();
                let password = secrets.admin_password.clone();
                rotate_password_in_db(&dbc, admin.id, &password).await?;
            }
            None => {
                // Empty user table: the ordinary seed provisions with these
                // credentials (it no-ops on a non-empty table, but then there
                // was no admin to recover either — a state we don't create).
                halogen_fixture::user::seed_admin_user(
                    &dbc,
                    &secrets.admin_username,
                    Some(&secrets.admin_password),
                )
                .await
                .map_err(|e| format!("Failed to seed admin during recovery: {e}"))?;
            }
        }
        let _ = dbc.close().await;
    }

    secrets.save(dirs)?;
    Ok(secrets.credentials())
}

/// Stop the server (if this process runs it), release the instance lock, and
/// delete the entire embedded root. A later [`ensure_started`] provisions a
/// fresh library. Errors if another process holds the library.
pub async fn destroy(dirs: &EmbeddedDirs) -> Result<(), String> {
    // Hold a lock handle through the removal so no other process can boot the
    // library out from under the delete.
    let _lock = match SHARED.get() {
        Some(shared) => {
            if shared.dirs != *dirs {
                return Err("Embedded server is running from a different directory".to_string());
            }
            stop().await;
            *shared.pending_socket.lock().expect("socket lock poisoned") = None;
            match shared
                .lock_file
                .lock()
                .expect("lock-file lock poisoned")
                .take()
            {
                Some(held) => Some(held),
                // We never held the single-instance lock (e.g. this process
                // failed to start because another instance owns the library).
                // Acquiring it must SUCCEED before we delete — a failure means a
                // live instance is still serving these files, so propagate the
                // error and refuse rather than `remove_dir_all` out from under it.
                None => Some(try_acquire_lock(dirs)?),
            }
        }
        None => {
            if !dirs.root().exists() {
                return Ok(());
            }
            Some(try_acquire_lock(dirs)?)
        }
    };

    let root = dirs.root().to_path_buf();
    if root.exists() {
        tokio::task::spawn_blocking(move || fs::remove_dir_all(&root))
            .await
            .map_err(|e| format!("Delete task failed: {e}"))?
            .map_err(|e| format!("Failed to delete embedded server data: {e}"))?;
    }
    Ok(())
}

fn base_url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn shared_or_init(dirs: EmbeddedDirs) -> Result<&'static Shared, String> {
    if let Some(shared) = SHARED.get() {
        if shared.dirs != dirs {
            return Err("Embedded server already bound to a different directory".to_string());
        }
        return Ok(shared);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("halogen-embedded")
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to build the embedded server runtime: {e}"))?;

    let socket = bind_loopback(None)?;
    let port = socket
        .local_addr()
        .map_err(|e| format!("Failed to read bound port: {e}"))?
        .port();
    info!("Embedded supervisor initialized (staged port {port})");

    let (status_tx, _) = watch::channel(EmbeddedStatus::Stopped);
    let (start_tx, start_rx) = mpsc::unbounded_channel();
    let (stop_gen, _) = watch::channel(0u64);
    let shared = Shared {
        dirs,
        runtime,
        status_tx,
        start_tx,
        stop_gen,
        port: Mutex::new(Some(port)),
        pending_socket: Mutex::new(Some(socket)),
        lock_file: Mutex::new(None),
    };

    if SHARED.set(shared).is_ok() {
        let shared = SHARED.get().expect("just set");
        shared.runtime.spawn(manager(shared, start_rx));
    }
    // On a lost set race our socket just drops; the winner's state rules.
    Ok(SHARED.get().expect("initialized above"))
}

impl Shared {
    fn status_now(&self) -> EmbeddedStatus {
        self.status_tx.borrow().clone()
    }

    /// `send_replace`, never `send`: the latter DROPS the value when nothing is
    /// subscribed, and this channel is also the status *store* ([`Self::status_now`]
    /// reads it back). A dropped `Starting` left `staged_port` seeing `Stopped`
    /// and handing out a second, unserved port.
    fn set_status(&self, status: EmbeddedStatus) {
        self.status_tx.send_replace(status);
    }

    /// The ONE bind-and-record site: previous-port-preferring bind, then the
    /// outcome recorded as the rebind target. Everything that needs a socket
    /// goes through here so the SO_REUSEADDR/port-persistence dance can't
    /// drift between `ensure_started`'s staging and the manager's runs.
    /// Reactor-free (bind is a plain syscall) — callable from any thread; the
    /// run converts to a listener on the supervisor runtime.
    fn bind_and_record(&self) -> Result<TcpSocket, String> {
        let previous = *self.port.lock().expect("port lock poisoned");
        let socket = bind_loopback(previous)?;
        let port = socket
            .local_addr()
            .map_err(|e| format!("Failed to read bound port: {e}"))?
            .port();
        *self.port.lock().expect("port lock poisoned") = Some(port);
        Ok(socket)
    }

    /// Port for the next/live run — staging a fresh socket if none is bound.
    ///
    /// The pending-socket lock is the synchronization point with the manager's
    /// [`Self::take_socket_for_run`]: `Starting` is published inside that same
    /// critical section, so this can never observe the torn middle of a start
    /// (pending already taken, status still `Stopped`). That window used to
    /// double-bind — the sticky port was still held by the taken socket, so
    /// this fell back to a fresh ephemeral one and handed out a URL no run
    /// would ever serve (the resolver then wrote it into the client config:
    /// every request hung against a socket nothing accepts).
    fn staged_port(&self) -> Result<u16, String> {
        let mut pending = self.pending_socket.lock().expect("socket lock poisoned");
        if let Some(socket) = pending.as_ref() {
            return socket
                .local_addr()
                .map(|a| a.port())
                .map_err(|e| format!("Failed to read staged port: {e}"));
        }
        match self.status_now() {
            EmbeddedStatus::Starting { port } | EmbeddedStatus::Running { port } => Ok(port),
            EmbeddedStatus::Stopped | EmbeddedStatus::Failed { .. } => {
                let socket = self.bind_and_record()?;
                let port = socket
                    .local_addr()
                    .map_err(|e| format!("Failed to read staged port: {e}"))?
                    .port();
                *pending = Some(socket);
                Ok(port)
            }
        }
    }

    /// The manager's per-run socket: the staged one when `ensure_started`
    /// bound ahead, else a fresh bind — publishing `Starting {{ port }}` while
    /// STILL holding the pending lock (see [`Self::staged_port`]). Never
    /// refuses on a live status: during an in-process restart turnaround the
    /// OLD run's `Running` is still published while the NEW run legitimately
    /// binds. The caller (on the supervisor runtime) converts the socket to a
    /// listener.
    fn take_socket_for_run(&self) -> Result<(TcpSocket, u16), String> {
        let mut pending = self.pending_socket.lock().expect("socket lock poisoned");
        let socket = match pending.take() {
            Some(socket) => socket,
            None => self.bind_and_record()?,
        };
        let port = socket
            .local_addr()
            .map_err(|e| format!("Failed to read bound port: {e}"))?
            .port();
        self.set_status(EmbeddedStatus::Starting { port });
        Ok((socket, port))
    }

    /// Acquire (once) and hold the single-instance lock.
    fn acquire_lock(&self) -> Result<(), String> {
        let mut guard = self.lock_file.lock().expect("lock-file lock poisoned");
        if guard.is_some() {
            return Ok(());
        }
        *guard = Some(try_acquire_lock(&self.dirs)?);
        Ok(())
    }
}

fn try_acquire_lock(dirs: &EmbeddedDirs) -> Result<fs::File, String> {
    fs::create_dir_all(dirs.root())
        .map_err(|e| format!("Failed to create {}: {e}", dirs.root().display()))?;
    let path = dirs.lock_path();
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
    // Contention and a failed lock syscall are different diagnoses — only the
    // former means another instance owns the library.
    match try_lock_exclusive(&file) {
        Ok(true) => Ok(file),
        Ok(false) => {
            Err("Another Halogen instance is already running the embedded server".to_string())
        }
        Err(e) => Err(format!("Failed to lock {}: {e}", path.display())),
    }
}

/// `Ok(false)` = another process holds it. Android has no `File::try_lock`
/// (std returns "not supported"), but bionic does have `flock(2)`.
#[cfg(target_os = "android")]
fn try_lock_exclusive(file: &fs::File) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) => Ok(false),
        _ => Err(err),
    }
}

#[cfg(not(target_os = "android"))]
fn try_lock_exclusive(file: &fs::File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(fs::TryLockError::WouldBlock) => Ok(false),
        Err(fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Bind loopback, preferring the previous port (SO_REUSEADDR — a graceful
/// drain leaves accepted sockets in TIME_WAIT on it), falling back to an
/// OS-assigned one.
fn bind_loopback(previous: Option<u16>) -> Result<TcpSocket, String> {
    if let Some(port) = previous
        && port != 0
        && let Ok(socket) = try_bind(port)
    {
        return Ok(socket);
    }
    if let Some(port) = previous {
        warn!("Embedded server port {port} unavailable — falling back to an ephemeral port");
    }
    try_bind(0).map_err(|e| format!("Failed to bind a loopback port: {e}"))
}

/// Bind only — `listen()` is deferred to the run (it registers with the
/// supervisor runtime's reactor; bind itself is reactor-free).
fn try_bind(port: u16) -> std::io::Result<TcpSocket> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.bind(SocketAddr::from(([127, 0, 0, 1], port)))?;
    Ok(socket)
}

async fn manager(shared: &'static Shared, mut start_rx: mpsc::UnboundedReceiver<u64>) {
    while let Some(first_gen) = start_rx.recv().await {
        // Coalesce queued pokes into one start, keeping the NEWEST stamp.
        let mut poke_gen = first_gen;
        while let Ok(g) = start_rx.try_recv() {
            poke_gen = poke_gen.max(g);
        }
        // The stop generation is sampled ONCE per start request (BEFORE the
        // staleness check — see below) and carried across every in-process
        // restart. Re-sampling per iteration would adopt a stop() that lands
        // in the between-runs window (after the old run's gen check, before
        // the new run's) as the new baseline — the drain would never fire and
        // stop() would wait forever.
        let run_gen = *shared.stop_gen.borrow();
        // Every coalesced poke predates the latest stop()/destroy(): a stopped
        // server must STAY stopped — the manager was blocked inside the run
        // while these queued (embedded config loads / auth calls poke on every
        // pass), and honoring them here silently rebooted it right after the
        // stop drained. Only an ensure_started issued after the stop (a newer
        // stamp) may boot. Sampling `run_gen` first also closes the race where
        // a stop lands between this check and the run: the run's baseline then
        // predates the bump, so its own gen check drains it immediately.
        if poke_gen < run_gen {
            continue;
        }
        if matches!(
            shared.status_now(),
            EmbeddedStatus::Starting { .. } | EmbeddedStatus::Running { .. }
        ) {
            continue;
        }
        if let Err(e) = shared.acquire_lock() {
            error!("Embedded server: {e}");
            shared.set_status(EmbeddedStatus::Failed { error: e });
            continue;
        }

        // The restart loop: RestartHandle re-enters (fresh overrides read),
        // stop()/failure exits back to waiting for the next start request.
        loop {
            if *shared.stop_gen.borrow() > run_gen {
                // Stopped between runs (or during a restart turnaround).
                info!("Embedded server stopped");
                shared.set_status(EmbeddedStatus::Stopped);
                break;
            }
            let (socket, port) = match shared.take_socket_for_run() {
                Ok(pair) => pair,
                Err(e) => {
                    error!("Embedded server: {e}");
                    shared.set_status(EmbeddedStatus::Failed { error: e });
                    break;
                }
            };
            info!("Embedded server starting on 127.0.0.1:{port}");

            match run_one(shared, socket, port, run_gen).await {
                RunEnd::Restart => {
                    info!("Embedded server restart requested — rebinding");
                    continue;
                }
                RunEnd::Stop => {
                    info!("Embedded server stopped");
                    shared.set_status(EmbeddedStatus::Stopped);
                    break;
                }
                RunEnd::Fatal(e) => {
                    error!("Embedded server failed: {e}");
                    shared.set_status(EmbeddedStatus::Failed { error: e });
                    break;
                }
            }
        }
    }
}

/// One server run: the same composition as `halogen-server`'s `main`
/// (config → migrate → seed → reclaim → polling → router → serve), with
/// library error handling instead of `process::exit`, and full teardown
/// (polling shutdown + pool close) once `serve` drains.
async fn run_one(shared: &'static Shared, socket: TcpSocket, port: u16, run_gen: u64) -> RunEnd {
    // First act on the supervisor runtime: turn the staged bound socket into
    // a listener (registers with THIS runtime's reactor).
    let listener = match socket.listen(1024) {
        Ok(listener) => listener,
        Err(e) => return RunEnd::Fatal(format!("Failed to listen on the staged socket: {e}")),
    };
    let dirs = &shared.dirs;

    let secrets = match Secrets::load_or_create(dirs) {
        Ok(secrets) => secrets,
        Err(e) => return RunEnd::Fatal(e),
    };
    let cfg = build_config(dirs, &secrets);
    log_config_overrides(&cfg);

    // Process-global SSRF guard — without this, outbound feed/art fetches are
    // unconfigured. Constant across restarts (not runtime-overridable).
    halogen_net::configure(cfg.allow_private_network);
    halogen_net::configure_user_agent(cfg.server_fetch_user_agent.clone());

    let dbc = match connect_and_migrate_wal(&cfg.db_path, !cfg.db_no_migrate).await {
        Ok(dbc) => dbc,
        Err(e) => return RunEnd::Fatal(format!("Database error: {e}")),
    };

    // Seed the admin (first boot only — the seed no-ops on a non-empty user
    // table) and the default "Queue" playlist.
    if let Err(e) = halogen_fixture::user::seed_admin_user(
        &dbc,
        &secrets.admin_username,
        Some(&secrets.admin_password),
    )
    .await
    {
        let _ = dbc.close().await;
        return RunEnd::Fatal(format!("Admin seed error: {e}"));
    }
    if !cfg.db_skip_default_playlist {
        match halogen_fixture::playlist::seed_default_queue(&dbc).await {
            Ok(true) => info!("Default \"Queue\" playlist created"),
            Ok(false) => {}
            Err(e) => warn!("Default playlist seed error: {e}"),
        }
    }

    match halogen_download::reclaim_orphaned_downloads(&dbc).await {
        Ok(n) if n > 0 => info!("Reset {n} orphaned download(s) on embedded startup"),
        Ok(_) => {}
        Err(e) => warn!("Failed to reclaim orphaned downloads on embedded startup: {e}"),
    }

    let polling = PollingHandle::from_config(dbc.clone(), &cfg);
    if let Err(e) = polling.start() {
        warn!("Failed to start embedded polling service: {e}");
    }

    let restart = RestartHandle::new();
    let router =
        halogen_server::routers::build_router(dbc.clone(), &cfg, polling.clone(), restart.clone());

    shared.set_status(EmbeddedStatus::Running { port });
    info!("Embedded server listening on 127.0.0.1:{port}");

    // Unlike the binary (which syncs before serving), a configured startup
    // sync runs detached — readiness must not wait on feed fetches.
    if cfg.subscription_sync_on_start {
        let polling = polling.clone();
        tokio::spawn(async move {
            match polling.poll().await {
                Ok(()) => info!("Embedded startup sync completed"),
                Err(e) => warn!("Embedded startup sync failed: {e}"),
            }
        });
    }

    // Drain on an API-requested restart or a host stop() (generational, so a
    // stop issued during init is seen immediately here).
    let shutdown = {
        let restart = restart.clone();
        let mut stop_rx = shared.stop_gen.subscribe();
        async move {
            let stopped = async {
                while *stop_rx.borrow_and_update() <= run_gen {
                    if stop_rx.changed().await.is_err() {
                        break;
                    }
                }
            };
            tokio::select! {
                _ = restart.wait() => {},
                _ = stopped => {},
            }
        }
    };

    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await;

    // Teardown before the next run (or idle): the poll loop must not outlive
    // this run's pool — an aborted mid-tick download is reclaimed on the next
    // start (same hardening as a process kill).
    polling.shutdown().await;
    let _ = dbc.close().await;

    if let Err(e) = serve_result {
        return RunEnd::Fatal(format!("Embedded server exited: {e}"));
    }
    if *shared.stop_gen.borrow() > run_gen {
        return RunEnd::Stop;
    }
    if restart.is_requested() {
        return RunEnd::Restart;
    }
    // serve() returned cleanly without either trigger — treat as stopped.
    RunEnd::Stop
}
