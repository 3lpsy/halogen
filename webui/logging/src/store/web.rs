//! Web device-log persistence: IndexedDB `halogen.logs`, object store `logs` with auto-increment keys. Each record is a
//! JSON-encoded [`LogLine`](super::super::LogLine) string. On append we prune the oldest records once the store exceeds
//! [`CAP`](super::super::CAP). The store is opened per call (device-log writes are infrequent and batched, so a cached
//! connection isn't worth the `Rc<RefCell>` dance the media store needs).

use anyhow::Result;
use idb::{Database, DatabaseEvent, Factory, ObjectStoreParams, TransactionMode};
use wasm_bindgen::JsValue;

use super::super::{CAP, LogLine};
use halogen_webui_store_idb::{commit_tx, idb_err, one_store_tx};

const DB_NAME: &str = "halogen.logs";
const DB_VERSION: u32 = 1;
const STORE: &str = "logs";

async fn db() -> Result<Database> {
    let factory = Factory::new().map_err(idb_err("indexeddb factory"))?;
    let mut open = factory
        .open(DB_NAME, Some(DB_VERSION))
        .map_err(idb_err("indexeddb open"))?;
    open.on_upgrade_needed(|event| {
        if let Ok(db) = event.database()
            && !db.store_names().iter().any(|n| n == STORE)
        {
            let mut params = ObjectStoreParams::new();
            params.auto_increment(true);
            let _ = db.create_object_store(STORE, params);
        }
    });
    open.await.map_err(idb_err("indexeddb open"))
}

/// Load persisted lines (oldest → newest), capped to the last `CAP`.
pub async fn load_all() -> Vec<LogLine> {
    load_all_inner().await.unwrap_or_default()
}

async fn load_all_inner() -> Result<Vec<LogLine>> {
    let db = db().await?;
    let (tx, store) = one_store_tx(&db, STORE, TransactionMode::ReadOnly)?;
    let values = store
        .get_all(None, None)
        .map_err(idb_err("get_all"))?
        .await
        .map_err(idb_err("get_all"))?;
    tx.await.map_err(idb_err("tx done"))?;
    let mut lines: Vec<LogLine> = values
        .into_iter()
        .filter_map(|v| v.as_string())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect();
    if lines.len() > CAP {
        lines.drain(0..lines.len() - CAP);
    }
    Ok(lines)
}

/// Append newly captured lines, then prune oldest records past `CAP`.
pub async fn append(new: &[LogLine]) {
    if new.is_empty() {
        return;
    }
    let _ = append_inner(new).await;
}

async fn append_inner(new: &[LogLine]) -> Result<()> {
    let db = db().await?;
    let (tx, store) = one_store_tx(&db, STORE, TransactionMode::ReadWrite)?;
    for line in new {
        let json = serde_json::to_string(line).map_err(idb_err("serialize"))?;
        let value = JsValue::from_str(&json);
        store
            .put(&value, None)
            .map_err(idb_err("put"))?
            .await
            .map_err(idb_err("put (quota?)"))?;
    }
    // Prune oldest keys beyond CAP (auto-increment keys come back ascending, so
    // the first `len - CAP` are the oldest).
    let keys = store
        .get_all_keys(None, None)
        .map_err(idb_err("keys"))?
        .await
        .map_err(idb_err("keys"))?;
    if keys.len() > CAP {
        let drop = keys.len() - CAP;
        for key in keys.into_iter().take(drop) {
            store
                .delete(key)
                .map_err(idb_err("delete"))?
                .await
                .map_err(idb_err("delete"))?;
        }
    }
    commit_tx(tx).await?;
    Ok(())
}

/// Delete every persisted record.
pub async fn clear() {
    let _ = clear_inner().await;
}

async fn clear_inner() -> Result<()> {
    let db = db().await?;
    let (tx, store) = one_store_tx(&db, STORE, TransactionMode::ReadWrite)?;
    store
        .clear()
        .map_err(idb_err("clear"))?
        .await
        .map_err(idb_err("clear"))?;
    commit_tx(tx).await?;
    Ok(())
}
