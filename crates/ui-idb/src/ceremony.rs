//! Low-level IndexedDB ceremony shared by every web store.
//!
//! Both the legacy hand-rolled stores (`media`, device `logging`) and the
//! `schema`/`json` layers in this crate repeat the same two bits of boilerplate at
//! every call: tagging each fallible step with a short context label, and opening a
//! single-store transaction. They live here so the stores carry only domain logic.
//!
//! (Moved verbatim from `halogen-ui-platform::idb` when the IndexedDB layer was
//! split into its own crate.)

use anyhow::{Result, anyhow};
use idb::{Database, ObjectStore, Transaction, TransactionMode};

/// Map an `idb` (or any `Display`) error to an `anyhow` error tagged with a short
/// context label — replaces the `|e| anyhow!("ctx: {e}")` closure that otherwise
/// repeats at every IndexedDB call site. (The `{e:?}` JsValue sites — object URL /
/// blob construction — keep their inline closures.)
pub fn idb_err<E: std::fmt::Display>(ctx: &'static str) -> impl FnOnce(E) -> anyhow::Error {
    move |e| anyhow!("{ctx}: {e}")
}

/// Open a transaction over a single object store and return both — the
/// `transaction(&[store]) + object_store(store)` pair every single-store op begins
/// with. Multi-store transactions still open inline.
pub fn one_store_tx(
    db: &Database,
    store: &str,
    mode: TransactionMode,
) -> Result<(Transaction, ObjectStore)> {
    let tx = db.transaction(&[store], mode).map_err(idb_err("tx"))?;
    let s = tx.object_store(store).map_err(idb_err("store"))?;
    Ok((tx, s))
}
