use halogen_wire::{
    SyncChangeData, SyncChangesData, SyncChangesParams, SyncResource, ValidationErrors,
};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, TransactionTrait};
use validator::Validate;

fn invalid_cursor() -> ValidationErrors {
    halogen_utils::verrors("cursor", "format", "Invalid sync cursor".into())
}

fn cursor_parts(cursor: &str) -> Result<(&str, i64), ValidationErrors> {
    let (epoch, sequence) = cursor.split_once(':').ok_or_else(invalid_cursor)?;
    if epoch.len() != 32
        || !epoch.bytes().all(|b| b.is_ascii_hexdigit())
        || sequence.is_empty()
        || !sequence.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid_cursor());
    }
    Ok((epoch, sequence.parse().map_err(|_| invalid_cursor())?))
}

fn statement(sql: &str, values: impl IntoIterator<Item = sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

/// Return changes for one authenticated actor; the cursor advances only through this read snapshot.
pub async fn changes(
    db: &DatabaseConnection,
    actor_id: i32,
    params: &SyncChangesParams,
) -> Result<SyncChangesData, ValidationErrors> {
    params.validate()?;
    if actor_id < 1 {
        return Err(halogen_utils::verrors(
            "actor",
            "range",
            "Invalid actor".into(),
        ));
    }
    let cursor = params.cursor.as_deref().map(cursor_parts).transpose()?;
    read_changes(db, actor_id, params.limit.unwrap_or(500), cursor)
        .await
        .map_err(|e| halogen_wire::DbValidationErrors::from(e).into())
}

async fn read_changes(
    db: &DatabaseConnection,
    actor_id: i32,
    limit: u64,
    cursor: Option<(&str, i64)>,
) -> Result<SyncChangesData, DbErr> {
    let txn = db.begin().await?;
    // Retain 180 days. Advancing the floor before pruning makes missed deletions force a snapshot.
    txn.execute_raw(statement("UPDATE sync_epoch SET floor_sequence = MAX(floor_sequence, COALESCE((SELECT MAX(sequence) FROM sync_change WHERE created_at < datetime('now', '-180 days')), 0)) WHERE id = 1", [])).await?;
    txn.execute_raw(statement("DELETE FROM sync_change WHERE sequence <= (SELECT floor_sequence FROM sync_epoch WHERE id = 1)", [])).await?;
    let state = txn.query_one_raw(statement("SELECT epoch, floor_sequence, COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'sync_change'), 0) AS head FROM sync_epoch WHERE id = 1", [])).await?
        .ok_or_else(|| DbErr::Custom("Sync epoch is missing".into()))?;
    let epoch: String = state.try_get("", "epoch")?;
    let floor: i64 = state.try_get("", "floor_sequence")?;
    let head: i64 = state.try_get("", "head")?;
    let reset = cursor.is_none_or(|(old_epoch, sequence)| {
        old_epoch != epoch || sequence < floor || sequence > head
    });
    if reset {
        txn.commit().await?;
        return Ok(SyncChangesData {
            changes: vec![],
            next_cursor: format!("{epoch}:{head}"),
            has_more: false,
            reset: true,
        });
    }
    let sequence = cursor.expect("non-reset cursor exists").1;
    let mut rows = txn.query_all_raw(statement(
        "SELECT sequence, resource, resource_id, deleted FROM sync_change WHERE actor_id = ? AND sequence > ? AND sequence <= ? ORDER BY sequence LIMIT ?",
        [actor_id.into(), sequence.into(), head.into(), (limit as i64 + 1).into()],
    )).await?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let mut changes = Vec::with_capacity(rows.len());
    for row in rows {
        let resource: String = row.try_get("", "resource")?;
        let resource = match resource.as_str() {
            "podcasts" => SyncResource::Podcasts,
            "episodes" => SyncResource::Episodes,
            "playbacks" => SyncResource::Playbacks,
            "playlists" => SyncResource::Playlists,
            "podcast_auto_playlists" => SyncResource::PodcastAutoPlaylists,
            _ => return Err(DbErr::Custom("Unknown sync resource".into())),
        };
        changes.push(SyncChangeData {
            sequence: row.try_get::<i64>("", "sequence")? as u64,
            resource,
            resource_id: row.try_get("", "resource_id")?,
            deleted: row.try_get("", "deleted")?,
        });
    }
    let next = if has_more {
        changes.last().expect("positive limit").sequence as i64
    } else {
        head
    };
    txn.commit().await?;
    Ok(SyncChangesData {
        changes,
        next_cursor: format!("{epoch}:{next}"),
        has_more,
        reset: false,
    })
}
