//! Shared playlist and membership ordering: calculate the target order, then rewrite contiguous 0..n positions in one
//! transaction. Only ActiveModel construction differs between entities.

use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
    TransactionTrait,
};

use crate::db_error;
use halogen_wire::ValidationErrors;

/// Move the element at index `from` to index `to` (clamped into range) within `ids`, returning the resulting
/// order. Returns `None` when it would be a no-op: an empty list, a `from` out of range, or a clamped `to` that
/// equals `from`. Pure (no DB) so the index math is unit-testable and identical for playlists and playlist
/// membership.
pub(crate) fn reordered(ids: &[i32], from: usize, to: i32) -> Option<Vec<i32>> {
    let len = ids.len();
    if from >= len {
        return None;
    }
    let to = to.clamp(0, len as i32 - 1) as usize;
    if to == from {
        return None;
    }
    let mut order = ids.to_vec();
    let moved = order.remove(from);
    order.insert(to, moved);
    Some(order)
}

/// Rewrite a set of rows' `position` to exactly match `ordered_ids` (contiguous 0..n) in a single transaction.
/// `build(id, new_position)` produces the `ActiveModel` for one row — typically the primary key `Unchanged`,
/// `position` `Set`, `updated_at` `Set`, everything else `NotSet`. Self-heals any holes left by deletes.
pub(crate) async fn rewrite_in_txn<A, F>(
    dbc: &DatabaseConnection,
    ordered_ids: &[i32],
    build: F,
) -> Result<(), ValidationErrors>
where
    A: ActiveModelTrait + ActiveModelBehavior + Send,
    <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
    F: Fn(i32, i32) -> A,
{
    write_positions_in_txn(dbc, None, ordered_ids, build).await
}

/// Like [`rewrite_in_txn`] but inserts `new_row` first, in the SAME transaction as the renumber. Use when
/// adding a row at a specific index: a renumber failure must not leave the freshly-inserted row committed at a
/// transient position. `ordered_ids` must already include the new row's id at its target index (the renumber
/// then places it, and everyone, at its final 0..n slot).
pub(crate) async fn insert_then_rewrite_in_txn<A, F>(
    dbc: &DatabaseConnection,
    new_row: A,
    ordered_ids: &[i32],
    build: F,
) -> Result<(), ValidationErrors>
where
    A: ActiveModelTrait + ActiveModelBehavior + Send,
    <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
    F: Fn(i32, i32) -> A,
{
    write_positions_in_txn(dbc, Some(new_row), ordered_ids, build).await
}

/// Shared core of [`rewrite_in_txn`] / [`insert_then_rewrite_in_txn`]: optionally
/// insert one new row, then rewrite every row in `ordered_ids` to its contiguous
/// 0..n `position`, all in one transaction.
async fn write_positions_in_txn<A, F>(
    dbc: &DatabaseConnection,
    new_row: Option<A>,
    ordered_ids: &[i32],
    build: F,
) -> Result<(), ValidationErrors>
where
    A: ActiveModelTrait + ActiveModelBehavior + Send,
    <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
    F: Fn(i32, i32) -> A,
{
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("opening transaction for reorder"))?;
    if let Some(row) = new_row {
        row.insert(&txn)
            .await
            .map_err(db_error("creating row during reorder"))?;
    }
    for (new_pos, id) in ordered_ids.iter().enumerate() {
        build(*id, new_pos as i32)
            .update(&txn)
            .await
            .map_err(db_error("updating position during reorder"))?;
    }
    txn.commit()
        .await
        .map_err(db_error("committing reorder transaction"))?;
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
