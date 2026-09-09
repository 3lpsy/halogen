use halogen_sync_enrich::{JournalEntry, OutboxOp, StoredEntry};

/// Only pending cursor saves are superseded; import receipts and rejected work survive.
pub(super) fn coalesce_cursor(
    transaction: &rusqlite::Transaction<'_>,
    operation: &OutboxOp,
) -> anyhow::Result<()> {
    let episode_id = match operation {
        OutboxOp::SetCursor { episode_id, .. } | OutboxOp::MarkPlayed { episode_id, .. } => {
            *episode_id
        }
        _ => return Ok(()),
    };
    let mut statement = transaction.prepare("SELECT id, op FROM outbox ORDER BY id")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, raw) in rows {
        let entry: JournalEntry = serde_json::from_str::<StoredEntry>(&raw)?.into();
        if entry.rejection.is_none()
            && !entry.delivered
            && matches!(entry.operation,
            OutboxOp::SetCursor { episode_id: previous, .. } if previous == episode_id)
        {
            transaction.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}
