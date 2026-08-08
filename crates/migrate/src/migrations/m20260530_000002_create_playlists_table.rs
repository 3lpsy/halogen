use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut user_fk = ForeignKey::create();
        user_fk
            .name("fk-playlist-user")
            .from_tbl(Playlist::Table)
            .from_col(Playlist::UserId)
            .to_tbl(User::Table)
            .to_col(User::Id)
            .on_delete(ForeignKeyAction::Cascade);

        manager
            .create_table(
                Table::create()
                    .table(Playlist::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Playlist::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Playlist::Name).string().not_null())
                    .col(ColumnDef::new(Playlist::Description).string())
                    // Owner: FK → user, CASCADE (declared below).
                    .col(ColumnDef::new(Playlist::UserId).integer().not_null())
                    .col(
                        ColumnDef::new(Playlist::IsDefault)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Playlist::CreatedAt)
                            .timestamp()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Playlist::UpdatedAt)
                            .timestamp()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(&mut user_fk)
                    .to_owned(),
            )
            .await?;

        // At most one default playlist PER USER: partial unique index on `user_id`
        // over rows where `is_default = 1`. The API clears the caller's prior default
        // in the same txn before promoting a new one, so this never trips in normal use.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_playlist_single_default_per_user \
                 ON playlist (user_id) WHERE is_default = 1",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Playlist::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum Playlist {
    Table,
    Id,
    Name,
    Description,
    UserId,
    IsDefault,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
#[allow(dead_code)]
enum User {
    Table,
    Id,
}
