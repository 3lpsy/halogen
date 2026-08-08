//! Web persistence for the device-global account registry: IndexedDB database
//! `halogen.accounts`, object store `registry`, a single `Durable` record at key
//! `"accounts"`. Connection cached for the process (single-threaded wasm).

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use idb::{Database, TransactionMode};

use halogen_ui_idb::{
    Schema, StoreKind, StoreSpec, commit_tx, get_json, idb_err, one_store_tx, open, put_json,
    str_key,
};
use halogen_ui_platform::store_keys::{ACCOUNTS_DB, ACCOUNTS_KEY, ACCOUNTS_STORE};

use crate::accounts::Accounts;

const SCHEMA_VERSION: u32 = 1;

const STORES: [StoreSpec; 1] = [StoreSpec {
    name: ACCOUNTS_STORE,
    auto_increment: false,
    kind: StoreKind::Durable,
    indexes: &[],
}];

thread_local! {
    static DB: RefCell<Option<Rc<Database>>> = const { RefCell::new(None) };
}

async fn db() -> Result<Rc<Database>> {
    if let Some(db) = DB.with(|d| d.borrow().clone()) {
        return Ok(db);
    }
    let schema = Schema {
        db_name: ACCOUNTS_DB.to_string(),
        version: SCHEMA_VERSION,
        stores: &STORES,
    };
    let db = Rc::new(open(&schema).await?.db);
    DB.with(|d| *d.borrow_mut() = Some(db.clone()));
    Ok(db)
}

/// Load the registry record, `None` when absent/unreadable.
pub async fn load() -> Option<Accounts> {
    load_inner().await.ok().flatten()
}

async fn load_inner() -> Result<Option<Accounts>> {
    let db = db().await?;
    let (tx, store) = one_store_tx(&db, ACCOUNTS_STORE, TransactionMode::ReadOnly)?;
    let value = get_json::<Accounts>(&store, str_key(ACCOUNTS_KEY)).await?;
    tx.await.map_err(idb_err("tx done"))?;
    Ok(value)
}

/// Persist the registry record (errors ignored — best-effort, as before).
pub async fn save(accounts: &Accounts) {
    let _ = save_inner(accounts).await;
}

async fn save_inner(accounts: &Accounts) -> Result<()> {
    let db = db().await?;
    let (tx, store) = one_store_tx(&db, ACCOUNTS_STORE, TransactionMode::ReadWrite)?;
    let key = str_key(ACCOUNTS_KEY);
    put_json(&store, Some(&key), accounts).await?;
    commit_tx(tx).await
}
