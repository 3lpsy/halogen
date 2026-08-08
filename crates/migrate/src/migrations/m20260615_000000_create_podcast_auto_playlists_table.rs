use super::common::TableWithTimestamps;
use super::m20250517_210000_create_podcasts_table::Podcast;
use super::m20260530_000002_create_playlists_table::Playlist;
use sea_orm_migration::prelude::*;

/// `podcast_auto_playlist` — junction: which playlist(s) a podcast auto-adds new
/// episodes to. Composite PK `(podcast_id, playlist_id)` doubles as the uniqueness
/// guard. FKs are `ON DELETE CASCADE` and enforced (`foreign_keys` is ON); delete
/// handlers also clean up explicitly. The `updated_at` trigger is installed by the
/// later `m20260616_000200_add_updated_at_triggers` (keyed by `rowid`, since this
/// table has no surrogate `id`).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut fk_podcast = ForeignKey::create();
        fk_podcast
            .name("fk-podcast_auto_playlist-podcast")
            .from_tbl(PodcastAutoPlaylist::Table)
            .from_col(PodcastAutoPlaylist::PodcastId)
            .to_tbl(Podcast::Table)
            .to_col(Podcast::Id)
            .on_delete(ForeignKeyAction::Cascade);

        let mut fk_playlist = ForeignKey::create();
        fk_playlist
            .name("fk-podcast_auto_playlist-playlist")
            .from_tbl(PodcastAutoPlaylist::Table)
            .from_col(PodcastAutoPlaylist::PlaylistId)
            .to_tbl(Playlist::Table)
            .to_col(Playlist::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(PodcastAutoPlaylist::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PodcastAutoPlaylist::PodcastId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PodcastAutoPlaylist::PlaylistId)
                            .integer()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(PodcastAutoPlaylist::PodcastId)
                            .col(PodcastAutoPlaylist::PlaylistId),
                    )
                    .add_timestamps()
                    .foreign_key(&mut fk_podcast)
                    .foreign_key(&mut fk_playlist)
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(PodcastAutoPlaylist::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum PodcastAutoPlaylist {
    Table,
    PodcastId,
    PlaylistId,
}
