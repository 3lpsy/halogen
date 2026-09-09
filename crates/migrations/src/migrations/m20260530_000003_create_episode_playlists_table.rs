use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut episode_fk = ForeignKey::create();
        episode_fk
            .name("fk-episode_playlist-episode")
            .from_tbl(EpisodePlaylist::Table)
            .from_col(EpisodePlaylist::EpisodeId)
            .to_tbl(Episode::Table)
            .to_col(Episode::Id)
            .on_delete(ForeignKeyAction::Cascade);

        let mut playlist_fk = ForeignKey::create();
        playlist_fk
            .name("fk-episode_playlist-playlist")
            .from_tbl(EpisodePlaylist::Table)
            .from_col(EpisodePlaylist::PlaylistId)
            .to_tbl(Playlist::Table)
            .to_col(Playlist::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(EpisodePlaylist::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(EpisodePlaylist::EpisodeId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EpisodePlaylist::PlaylistId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(EpisodePlaylist::Position)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(EpisodePlaylist::CreatedAt)
                            .timestamp()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(EpisodePlaylist::UpdatedAt)
                            .timestamp()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(&mut episode_fk)
                    .foreign_key(&mut playlist_fk)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(EpisodePlaylist::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum EpisodePlaylist {
    Table,
    EpisodeId,
    PlaylistId,
    Position,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
#[allow(dead_code)]
enum Episode {
    Table,
    Id,
}

#[derive(Iden)]
#[allow(dead_code)]
enum Playlist {
    Table,
    Id,
}
