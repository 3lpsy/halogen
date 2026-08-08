use sea_orm_migration::prelude::*;

/// Backfill `user_podcast` subscriptions for podcasts that predate the
/// subscription table (`m20260616_000000_create_user_podcasts_table`).
///
/// That migration created the junction empty, so every podcast added before it
/// has no subscriber — and once the episode/podcast list endpoints became
/// subscription-scoped, their owners stopped seeing their own library. Going
/// forward `POST /podcasts` auto-subscribes the creator (`podcast_store`); this
/// closes the gap for existing rows by subscribing each podcast's `owner_id`.
///
/// `INSERT OR IGNORE` makes it idempotent against the composite PK
/// `(user_id, podcast_id)` — podcasts already auto-subscribed are skipped. The
/// subscription's timestamps are copied from the podcast's own `created_at` /
/// `updated_at`, which both avoids re-deriving a timestamp in the migration and
/// stores them in the exact text format the rest of the table uses.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // ── Defensive repair for the one-migration-per-change invariant being
        // violated upstream ──────────────────────────────────────────────────
        // `podcast.owner_id`, `episode.guid`, and `episode.download_size` were
        // added INLINE to their shipped `create_*` migrations instead of via
        // dedicated ALTERs. A DB created at an intermediate state therefore
        // reaches this point WITHOUT them: the `SELECT owner_id` below would error
        // (halting migrations, so the server refuses to start), and even past
        // migration the entities' SELECTs of `guid`/`download_size` would fail. A
        // fresh/current DB already has these from the create migrations, so each
        // add is guarded by `has_column` and is a no-op there (and a DB that
        // already applied this migration records it by name and never re-runs).
        if !manager.has_column("podcast", "owner_id").await? {
            conn.execute_unprepared(
                "ALTER TABLE podcast ADD COLUMN owner_id INTEGER NOT NULL DEFAULT 0",
            )
            .await?;
            // Give existing rows a valid owner (an admin, else any user) so the
            // backfilled subscriptions satisfy the owner FK.
            conn.execute_unprepared(
                "UPDATE podcast SET owner_id = COALESCE(\
                     (SELECT id FROM user WHERE is_admin = 1 ORDER BY id LIMIT 1),\
                     (SELECT id FROM user ORDER BY id LIMIT 1),\
                     owner_id)",
            )
            .await?;
        }
        if !manager.has_column("episode", "guid").await? {
            conn.execute_unprepared("ALTER TABLE episode ADD COLUMN guid TEXT")
                .await?;
        }
        if !manager.has_column("episode", "download_size").await? {
            conn.execute_unprepared("ALTER TABLE episode ADD COLUMN download_size INTEGER")
                .await?;
        }

        conn.execute_unprepared(
            "INSERT OR IGNORE INTO user_podcast (user_id, podcast_id, created_at, updated_at) \
             SELECT owner_id, id, created_at, updated_at FROM podcast",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Pure data backfill: the inserted owner subscriptions are
        // indistinguishable from ones the app would create normally, so there is
        // nothing safe to undo. No-op (the table itself is dropped by the create
        // migration's `down`).
        Ok(())
    }
}
