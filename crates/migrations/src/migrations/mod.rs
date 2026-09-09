pub mod m20260908_000100_create_sync_epoch_table;
pub mod m20260908_000200_create_sync_changes_table;
pub use sea_orm_migration::prelude::*;
pub mod common;

pub mod m20250117_000001_create_users_table;
pub mod m20250517_210000_create_podcasts_table;
pub mod m20250517_230000_create_episodes_table;
pub mod m20260529_000000_create_playback_table;
pub mod m20260530_000002_create_playlists_table;
pub mod m20260530_000003_create_episode_playlists_table;
pub mod m20260614_000000_create_podcast_configs_table;
pub mod m20260615_000000_create_podcast_auto_playlists_table;
pub mod m20260616_000000_create_user_podcasts_table;
pub mod m20260616_000100_create_user_episode_statuses_table;
pub mod m20260616_000200_add_updated_at_triggers;
pub mod m20260616_000300_add_position_to_playlists_table;
pub mod m20260617_000000_backfill_user_podcast_subscriptions;
pub mod m20260617_000100_backfill_user_episode_status_from_playback;
pub mod m20260617_000200_add_feed_url_redirects_to_podcasts_table;
pub mod m20260617_000300_unique_episode_playlist;
pub mod m20260619_000000_add_download_columns_to_episodes_table;
pub mod m20260619_000100_create_episode_chapters_table;
pub mod m20260701_000100_create_poll_job_table;
pub mod m20260701_000200_create_poll_job_podcast_table;
pub mod m20260701_000300_create_podcast_sync_error_table;
pub mod m20260701_000400_create_episode_download_error_table;
pub mod m20260717_000100_add_add_to_start_to_podcast_auto_playlists_table;
pub mod m20260720_000100_add_on_remove_delete_flags_to_playlists_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250117_000001_create_users_table::Migration),
            // podcast_config before podcast: `podcast.podcast_config_id` FK
            // references it, so the parent table must exist first.
            Box::new(m20260614_000000_create_podcast_configs_table::Migration),
            Box::new(m20250517_210000_create_podcasts_table::Migration),
            Box::new(m20250517_230000_create_episodes_table::Migration),
            Box::new(m20260529_000000_create_playback_table::Migration),
            Box::new(m20260530_000002_create_playlists_table::Migration),
            Box::new(m20260530_000003_create_episode_playlists_table::Migration),
            Box::new(m20260615_000000_create_podcast_auto_playlists_table::Migration),
            Box::new(m20260616_000000_create_user_podcasts_table::Migration),
            Box::new(m20260616_000100_create_user_episode_statuses_table::Migration),
            // Backfill `updated_at` triggers on the tables created without one;
            // must run after all those tables exist.
            Box::new(m20260616_000200_add_updated_at_triggers::Migration),
            // Add `playlist.position` (manual order) + backfill existing rows.
            Box::new(m20260616_000300_add_position_to_playlists_table::Migration),
            // Subscribe existing podcasts' owners (the subscription table was
            // created empty; scoping then hid pre-existing libraries from owners).
            Box::new(m20260617_000000_backfill_user_podcast_subscriptions::Migration),
            // Reconcile pre-existing playbacks into per-user listen status (the
            // status table was created empty; finished/started episodes showed
            // Unplayed and mis-filtered).
            Box::new(m20260617_000100_backfill_user_episode_status_from_playback::Migration),
            // Add `podcast.feed_url_redirects` (last-poll redirect hop chain, CSV).
            Box::new(m20260617_000200_add_feed_url_redirects_to_podcasts_table::Migration),
            // Dedupe + add a unique index on episode_playlist(episode_id, playlist_id).
            Box::new(m20260617_000300_unique_episode_playlist::Migration),
            // Add `episode.download_started_at` + `episode.download_attempts`
            // (stuck-download watchdog + bounded auto-retry bookkeeping).
            Box::new(m20260619_000000_add_download_columns_to_episodes_table::Migration),
            // Read-only per-episode chapter markers (psc:chapters / podcast:chapters),
            // populated best-effort during feed sync.
            Box::new(m20260619_000100_create_episode_chapters_table::Migration),
            // Durable sync-run history: poll_job before its child outcome table
            // (the FK needs the parent first).
            Box::new(m20260701_000100_create_poll_job_table::Migration),
            Box::new(m20260701_000200_create_poll_job_podcast_table::Migration),
            // Failure-reason history tables (RSS sync per podcast, media download
            // per episode) — after podcast/episode exist (FKs).
            Box::new(m20260701_000300_create_podcast_sync_error_table::Migration),
            Box::new(m20260701_000400_create_episode_download_error_table::Migration),
            // Add `podcast_auto_playlist.add_to_start` (per-link insert-position
            // override for auto-added episodes; NULL = server default).
            Box::new(m20260717_000100_add_add_to_start_to_podcast_auto_playlists_table::Migration),
            // Add `playlist.on_remove_delete_file_{server,client}` (per-playlist
            // delete-download-on-remove cleanup flags, default FALSE).
            Box::new(m20260720_000100_add_on_remove_delete_flags_to_playlists_table::Migration),
            Box::new(m20260908_000100_create_sync_epoch_table::Migration),
            Box::new(m20260908_000200_create_sync_changes_table::Migration),
        ]
    }
}
