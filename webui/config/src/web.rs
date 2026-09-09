//! Web persistence backend for the per-user config stores: IndexedDB database `halogen.config.{segment}`, a single `kv`
//! object store holding one record per suffix (`"client_config"`, `"list_views"`). The store is `Durable`, never wiped
//! on a schema bump (config is not re-fetchable). Connections are cached per database (single-threaded wasm), so the
//! frequent settled config writes don't reopen each time.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use idb::{Database, TransactionMode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use halogen_webui_platform::store_keys::{CONFIG_DB_PREFIX, CONFIG_STORE};
use halogen_webui_store_idb::{
    Schema, StoreKind, StoreSpec, commit_tx, delete, get_json, idb_err, one_store_tx, open,
    put_json, str_key,
};

/// Bump to evolve the config schema on a new release; the `Durable` store's data
/// survives (only an explicit migration would reshape records).
const SCHEMA_VERSION: u32 = 1;

const STORES: [StoreSpec; 1] = [StoreSpec {
    name: CONFIG_STORE,
    auto_increment: false,
    kind: StoreKind::Durable,
    indexes: &[],
}];

thread_local! {
    /// Open config databases keyed by name (`halogen.config.{segment}`).
    static DBS: RefCell<HashMap<String, Rc<Database>>> = RefCell::new(HashMap::new());
}

async fn db(segment: &str) -> Result<Rc<Database>> {
    let name = format!("{CONFIG_DB_PREFIX}{segment}");
    if let Some(db) = DBS.with(|m| m.borrow().get(&name).cloned()) {
        return Ok(db);
    }
    let schema = Schema {
        db_name: name.clone(),
        version: SCHEMA_VERSION,
        stores: &STORES,
    };
    let db = Rc::new(open(&schema).await?.db);
    DBS.with(|m| m.borrow_mut().insert(name, db.clone()));
    Ok(db)
}

/// Load the record at `key` for `segment`. A backend failure (DB won't open,
/// bad transaction, corrupt record) is an `Err` — only a genuinely absent
/// record is `Ok(None)`, so "couldn't read" can't silently become "use
/// defaults" (the store layer decides how to degrade).
pub async fn try_load<T: DeserializeOwned>(segment: &str, key: &str) -> Result<Option<T>, String> {
    load_inner(segment, key).await.map_err(|e| e.to_string())
}

async fn load_inner<T: DeserializeOwned>(segment: &str, key: &str) -> Result<Option<T>> {
    let db = db(segment).await?;
    let (tx, store) = one_store_tx(&db, CONFIG_STORE, TransactionMode::ReadOnly)?;
    let value = get_json::<T>(&store, str_key(key)).await?;
    tx.await.map_err(idb_err("tx done"))?;
    Ok(value)
}

/// Persist `value` at `key` for `segment` (errors ignored, matching the old
/// best-effort localStorage write).
pub async fn save<T: Serialize>(segment: &str, key: &str, value: &T) {
    let _ = save_inner(segment, key, value).await;
}

async fn save_inner<T: Serialize>(segment: &str, key: &str, value: &T) -> Result<()> {
    let db = db(segment).await?;
    let (tx, store) = one_store_tx(&db, CONFIG_STORE, TransactionMode::ReadWrite)?;
    let k = str_key(key);
    put_json(&store, Some(&k), value).await?;
    commit_tx(tx).await
}

/// Delete the record at `key` for `segment`.
pub async fn clear(segment: &str, key: &str) {
    let _ = clear_inner(segment, key).await;
}

async fn clear_inner(segment: &str, key: &str) -> Result<()> {
    let db = db(segment).await?;
    let (tx, store) = one_store_tx(&db, CONFIG_STORE, TransactionMode::ReadWrite)?;
    delete(&store, str_key(key)).await?;
    commit_tx(tx).await
}
