use sea_orm_migration::prelude::*;

/// Backfill `user_episode_status` from existing `playback` rows.
///
/// Per-user listen state (`Unplayed`/`Played`/`Finished`) moved into
/// `user_episode_status` (`m20260616_000100`), created empty. The playback upsert
/// keeps the two in sync going forward (`playback_store::playback_status_for`),
/// but playbacks recorded before that table existed have no status row — so
/// episodes a user had finished/started showed as `Unplayed` and were mis-filtered
/// by the listen-state filter. (The `playback.completed`/`cursor` data itself was
/// never lost; only the derived per-user status was missing.)
///
/// Mapping mirrors the handler with the data available in SQL:
///   - `completed = 1`            → `FINISHED`
///   - else `cursor > 0`          → `PLAYED`
///   - else (never started)       → no row (`UNPLAYED` is the absent-row default)
///
/// The handler also promotes "played into the last N%" to `FINISHED`, but that
/// needs the episode duration + config percentage; an un-flagged near-end playback
/// backfills as `PLAYED` and is re-derived to `FINISHED` on the next playback
/// write. `INSERT OR IGNORE` keys off the unique `(user_id, episode_id)` index so
/// it's idempotent and never clobbers a status the app already wrote. Timestamps
/// are copied from the playback row.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT OR IGNORE INTO user_episode_status \
                     (user_id, episode_id, playback_status, created_at, updated_at) \
                 SELECT user_id, episode_id, \
                        CASE WHEN completed = 1 THEN 'FINISHED' ELSE 'PLAYED' END, \
                        created_at, updated_at \
                 FROM playback \
                 WHERE completed = 1 OR cursor > 0",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Pure data backfill: the inserted statuses are indistinguishable from ones
        // the playback handler would write, so there is nothing safe to undo. No-op
        // (the table itself is dropped by the create migration's `down`).
        Ok(())
    }
}
