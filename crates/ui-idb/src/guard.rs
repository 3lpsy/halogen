//! Drop-safe await wrappers around `idb` requests / transactions.
//!
//! `idb` 0.6.5 registers an `IdbRequest`'s `onsuccess`/`onerror` (and a
//! transaction's `oncomplete`/`onerror`/`onabort`) as wasm-bindgen `Closure`s it
//! stores INSIDE the request/transaction future — but it never clears those
//! handlers when that future is DROPPED before completion. So if a caller's task is
//! cancelled mid-flight (a Dioxus `use_future`/`spawn` re-running or unmounting —
//! the common case for our IndexedDB reads, e.g. the episode-list `read_cache`), the
//! `Closure` is freed while the live `IdbRequest` still points at it; when the
//! browser later fires the event it invokes a dropped closure and wasm-bindgen
//! throws "closure invoked recursively or after being dropped", surfacing as an
//! uncaught error from `IDBRequest`/`IDBTransaction`.
//!
//! These combinators wrap the await with a RAII guard that clears the handler on
//! drop, so a cancelled request/transaction becomes inert instead of panicking. The
//! clear runs synchronously during the cancellation drop — before control returns
//! to the JS event loop — so even though `idb`'s closure is freed in the same drop,
//! the browser can never fire it (the handler is already null).
//!
//! [`guarded`] returns the SAME type the bare `idb` future would, so call sites only
//! wrap the request: `guarded(store.get(k)?).await` for `store.get(k)?.await`.

use std::future::{Future, IntoFuture};

use anyhow::Result;
use idb::Transaction;
use web_sys::{IdbRequest, IdbTransaction};

use crate::ceremony::idb_err;

/// Clears an `IdbRequest`'s `onsuccess`/`onerror` on drop (see module docs).
struct ClearRequest(IdbRequest);

impl Drop for ClearRequest {
    fn drop(&mut self) {
        self.0.set_onsuccess(None);
        self.0.set_onerror(None);
    }
}

/// Await any `idb` store/index/cursor request drop-safely. Works for every request
/// type (`get`/`get_all`/`get_all_keys`/`put`/`add`/`delete`/`clear`/`count`/
/// `open_cursor` + cursor `advance`/`next`) because they all convert to/from
/// `web_sys::IdbRequest` and implement `IntoFuture`. The output is exactly the bare
/// future's output, so callers keep their existing result handling + `.map_err`.
pub async fn guarded<R>(request: R) -> <R::IntoFuture as Future>::Output
where
    R: Into<IdbRequest> + From<IdbRequest> + IntoFuture,
{
    let raw: IdbRequest = request.into();
    // The guard owns a handle to the same underlying request; on drop (including a
    // cancellation drop at the await below) it nulls the handlers, all within the
    // synchronous drop — so a freed `idb` closure can never be fired by the browser.
    let _clear = ClearRequest(raw.clone());
    R::from(raw).await
}

/// Clears a transaction's `oncomplete`/`onerror`/`onabort` on drop (see module docs).
struct ClearTx(IdbTransaction);

impl Drop for ClearTx {
    fn drop(&mut self) {
        self.0.set_oncomplete(None);
        self.0.set_onerror(None);
        self.0.set_onabort(None);
    }
}

/// Commit a transaction and await its completion drop-safely — the guarded
/// replacement for the `tx.commit()?.await?` every write op ends with.
pub async fn commit_tx(tx: Transaction) -> Result<()> {
    let committed = tx.commit().map_err(idb_err("commit"))?;
    await_tx(committed, "commit").await
}

/// Await a (read-only) transaction's completion drop-safely — the guarded
/// replacement for the bare `tx.await` a read op ends with to flush the transaction.
pub async fn tx_done(tx: Transaction) -> Result<()> {
    await_tx(tx, "tx done").await
}

async fn await_tx(tx: Transaction, ctx: &'static str) -> Result<()> {
    let raw: IdbTransaction = tx.into();
    let _clear = ClearTx(raw.clone());
    Transaction::from(raw).await.map_err(idb_err(ctx))?;
    Ok(())
}
