//! Wrap idb futures with drop guards that synchronously clear browser handlers before cancellation frees their
//! closures. This prevents late events from invoking dropped wasm closures. [`guarded`] preserves the wrapped future's
//! result type.

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

/// Await any `idb` store/index/cursor request drop-safely. Works for every request type
/// (`get`/`get_all`/`get_all_keys`/`put`/`add`/`delete`/`clear`/`count`/ `open_cursor` + cursor `advance`/`next`)
/// because they all convert to/from `web_sys::IdbRequest` and implement `IntoFuture`. The output is exactly the bare
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
