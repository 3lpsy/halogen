//! Open a declared [`Schema`]; version upgrades recreate `Cache` stores and preserve existing `Durable` stores. Bump
//! `version` for structural changes. Incompatible durable records need an async transform after [`open`], keyed by
//! [`Opened::previous_version`], because the upgrade callback is synchronous.

use std::cell::Cell;
use std::future::IntoFuture;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use futures_util::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use idb::{Database, DatabaseEvent, Factory, KeyPath, ObjectStoreParams};
use wasm_bindgen::JsCast;

use crate::ceremony::idb_err;

/// Bound waits only after IndexedDB reports `blocked`, usually from another open connection. Slow but unblocked
/// operations remain untimed; otherwise an old tab could hang every new-context store call indefinitely.
const BLOCKED_TIMEOUT_MS: u32 = 15_000;

/// Await an idb request future, but once the request has reported `blocked`
/// (per `blocked_flag`), give it at most [`BLOCKED_TIMEOUT_MS`] per check
/// before failing with a "close other tabs" error instead of hanging forever.
async fn await_unless_blocked<T>(
    fut: impl Future<Output = std::result::Result<T, idb::Error>>,
    blocked_flag: Rc<Cell<bool>>,
    what: &'static str,
    db_name: &str,
) -> Result<T> {
    let mut fut = Box::pin(fut);
    loop {
        match select(fut, TimeoutFuture::new(BLOCKED_TIMEOUT_MS)).await {
            Either::Left((res, _)) => return res.map_err(idb_err(what)),
            Either::Right(((), rest)) => {
                if blocked_flag.get() {
                    return Err(anyhow!(
                        "indexeddb {what} {db_name}: blocked by another connection for over \
                         {BLOCKED_TIMEOUT_MS}ms — an older app tab/window holds this database \
                         open; close other tabs of this app"
                    ));
                }
                // Not blocked, just slow (e.g. deleting a large database) —
                // keep waiting.
                fut = rest;
            }
        }
    }
}

/// Arm a `blocked` flag + console diagnostic on an open/delete request. The
/// console (not `halogen-webui-logging`, which depends on this crate) is the only
/// sink available down here; [`await_unless_blocked`] carries the same message
/// into the returned error for the callers that do log.
fn on_blocked_flag(
    flag: Rc<Cell<bool>>,
    what: &'static str,
    db_name: &str,
) -> impl FnOnce() + use<> {
    let msg = format!(
        "indexeddb {what} {db_name}: blocked — another tab/worker holds an older version open"
    );
    move || {
        flag.set(true);
        web_sys::console::warn_1(&msg.into());
    }
}

/// Close on `versionchange` so old tabs cannot indefinitely block upgrades or deletion. This context's later calls fail
/// visibly instead of preventing the newer context from opening its database.
fn close_on_version_change(db: &mut Database, db_name: &str) {
    let msg = format!(
        "indexeddb {db_name}: closing — another tab/worker requested a version change \
         (app update or storage wipe); reload this tab"
    );
    db.on_version_change(move |event: web_sys::Event| {
        if let Some(target) = event.target()
            && let Ok(db) = target.dyn_into::<web_sys::IdbDatabase>()
        {
            db.close();
        }
        web_sys::console::warn_1(&msg.into());
    });
}

/// What an object store holds — decides its upgrade policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    /// Re-fetchable cache: dropped + recreated on every version bump.
    Cache,
    /// Irreplaceable data: created if missing, otherwise preserved across upgrades.
    Durable,
}

/// One IndexedDB index on a store: a `name` and the `key_path` (a property of the
/// stored value object) it sorts by. Records whose value lacks the `key_path`
/// property are simply absent from the index, so a record meant to be pageable by
/// this index must always carry a present, non-`undefined` value there.
#[derive(Clone, Copy, Debug)]
pub struct IndexSpec {
    /// Index name (passed to `ObjectStore::index`).
    pub name: &'static str,
    /// Single-property key path the index sorts by (e.g. `"pub"`).
    pub key_path: &'static str,
}

/// One object store in a [`Schema`].
#[derive(Clone, Copy, Debug)]
pub struct StoreSpec {
    /// Object-store name.
    pub name: &'static str,
    /// `true` for an auto-increment key store (the outbox); `false` for explicit
    /// out-of-line keys (everything keyed by an id/suffix the caller supplies).
    pub auto_increment: bool,
    /// Cache (droppable) vs durable (preserved) — see [`StoreKind`].
    pub kind: StoreKind,
    /// Indexes to (re)create alongside the store. Empty for stores whose value is
    /// an opaque JSON string (an index key path can't see into it). A `Cache`
    /// store's indexes are rebuilt for free on each version bump (drop + recreate).
    pub indexes: &'static [IndexSpec],
}

/// A database's full shape: its name, schema version, and object stores.
pub struct Schema {
    /// Database name (often built per-user, e.g. `halogen.store.u3`).
    pub db_name: String,
    /// Schema version — bump to trigger an upgrade on the next open.
    pub version: u32,
    /// Every object store the database should contain.
    pub stores: &'static [StoreSpec],
}

/// The result of [`open`]: the connection plus the on-disk version that preceded
/// this open (`< schema.version` exactly when an upgrade just ran — `0` on first
/// create). Callers needing an async `Durable`-record migration branch on it.
pub struct Opened {
    pub db: Database,
    pub previous_version: u32,
}

/// Open `schema`'s database at its version, reconciling object stores per their
/// [`StoreKind`] on upgrade. Self-heals one missing-store case (see below).
pub async fn open(schema: &Schema) -> Result<Opened> {
    let factory = Factory::new().map_err(idb_err("indexeddb factory"))?;
    let opened = open_once(&factory, schema).await?;
    if all_stores_present(&opened.db, schema) {
        return Ok(opened);
    }

    // A store is missing even though the DB sits at the target version, an interrupted upgrade, or a pre-framework DB
    // of the same name/version. Delete and recreate cleanly at the SAME version (no version drift, unlike bumping to
    // version+1). Cache content re-pulls; a durable store that was already missing was unrecoverable anyway, so
    // defaults are the only outcome.
    opened.db.close();
    delete_db(&schema.db_name).await?;
    let opened = open_once(&factory, schema).await?;
    if !all_stores_present(&opened.db, schema) {
        return Err(anyhow!(
            "indexeddb {}: object stores missing after repair",
            schema.db_name
        ));
    }
    Ok(opened)
}

/// Open once at `schema.version`, applying the structural upgrade in the callback.
async fn open_once(factory: &Factory, schema: &Schema) -> Result<Opened> {
    // Default to "no upgrade ran" (already at target version); the callback
    // overwrites with the real prior version when an upgrade fires.
    let previous = Rc::new(Cell::new(schema.version));
    let previous_cb = previous.clone();
    let stores = schema.stores;

    let mut request = factory
        .open(&schema.db_name, Some(schema.version))
        .map_err(idb_err("indexeddb open"))?;
    request.on_upgrade_needed(move |event| {
        if let Ok(old) = event.old_version() {
            previous_cb.set(old);
        }
        if let Ok(db) = event.database() {
            apply_structure(&db, stores);
        }
    });
    let blocked = Rc::new(Cell::new(false));
    let on_blocked = on_blocked_flag(blocked.clone(), "open", &schema.db_name);
    request.on_blocked(move |_| on_blocked());
    let mut db =
        await_unless_blocked(request.into_future(), blocked, "open", &schema.db_name).await?;
    close_on_version_change(&mut db, &schema.db_name);
    Ok(Opened {
        db,
        previous_version: previous.get(),
    })
}

/// Reconcile object stores inside the (synchronous) upgrade transaction. Decisions
/// are made against the snapshot taken at entry, so the distinct store names don't
/// interfere as we create/drop them.
fn apply_structure(db: &Database, stores: &[StoreSpec]) {
    let existing = db.store_names();
    for spec in stores {
        let present = existing.iter().any(|n| n == spec.name);
        match spec.kind {
            // Cache: always end at a fresh, empty store (drop the stale one first).
            StoreKind::Cache => {
                if present {
                    let _ = db.delete_object_store(spec.name);
                }
                create_store(db, spec);
            }
            // Durable: create only when missing; never touch existing data.
            StoreKind::Durable => {
                if !present {
                    create_store(db, spec);
                }
            }
        }
    }
}

fn create_store(db: &Database, spec: &StoreSpec) {
    let mut params = ObjectStoreParams::new();
    if spec.auto_increment {
        params.auto_increment(true);
    }
    // No key_path: every non-autoincrement store uses explicit out-of-line keys
    // (the value is an opaque JSON string, or — for indexed stores — an object
    // whose index key paths sit beside that string). A failure leaves the store
    // missing, which `open`'s self-heal detects and repairs.
    let Ok(store) = db.create_object_store(spec.name, params) else {
        return;
    };
    // Indexes are part of the store's structure, so they're (re)created here in the
    // same upgrade transaction. `None` params → a plain, non-unique index.
    for idx in spec.indexes {
        let _ = store.create_index(idx.name, KeyPath::new_single(idx.key_path), None);
    }
}

fn all_stores_present(db: &Database, schema: &Schema) -> bool {
    let names = db.store_names();
    schema
        .stores
        .iter()
        .all(|spec| names.iter().any(|n| n == spec.name))
}

/// Open `db_name` at its current on-disk version **without** the schema framework (no upgrade callback, no store
/// creation). For read-only access by tools that don't own the schema, the `/cache-control` failsafe
/// enumerating/clearing stores. NOTE: IndexedDB has no "open only if it exists", so a missing database is created empty
/// (with no object stores), callers skip missing stores.
pub async fn open_current(db_name: &str) -> Result<Database> {
    let factory = Factory::new().map_err(idb_err("indexeddb factory"))?;
    let mut request = factory
        .open(db_name, None)
        .map_err(idb_err("indexeddb open"))?;
    // A version-free open can't itself be blocked by an upgrade, but the
    // connection it returns can block LATER upgrades/deletes — same
    // versionchange discipline as `open`.
    let blocked = Rc::new(Cell::new(false));
    let on_blocked = on_blocked_flag(blocked.clone(), "open", db_name);
    request.on_blocked(move |_| on_blocked());
    let mut db = await_unless_blocked(request.into_future(), blocked, "open", db_name).await?;
    close_on_version_change(&mut db, db_name);
    Ok(db)
}

/// Delete a database outright (sign-out wipe / the `/cache-control` failsafe).
/// Succeeds even if the database doesn't exist.
pub async fn delete_db(name: &str) -> Result<()> {
    let factory = Factory::new().map_err(idb_err("indexeddb factory"))?;
    let mut request = factory.delete(name).map_err(idb_err("indexeddb delete"))?;
    // `deleteDatabase` stalls indefinitely while ANY other connection stays
    // open (the main thread vs the sync worker during self-heal, another tab)
    // — fail after the blocked window instead of hanging the wipe forever.
    // Our own connections close themselves via `close_on_version_change`.
    let blocked = Rc::new(Cell::new(false));
    let on_blocked = on_blocked_flag(blocked.clone(), "delete", name);
    request.on_blocked(move |_| on_blocked());
    await_unless_blocked(request.into_future(), blocked, "delete", name).await?;
    Ok(())
}
