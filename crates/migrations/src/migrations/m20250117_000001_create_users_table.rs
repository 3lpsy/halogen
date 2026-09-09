use super::common::{MigrationTimestampExt, TableWithTimestamps};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(User::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(User::Id)
                            .integer()
                            .auto_increment()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(User::Username)
                            .string()
                            .not_null()
                            .unique_key(),
                    )
                    .col(ColumnDef::new(User::PasswordHash).string().not_null())
                    .col(
                        ColumnDef::new(User::IsAdmin)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .add_timestamps()
                    .to_owned(),
            )
            .await?;
        self.create_timestamp_trigger(manager, User::Table.to_string())
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        self.drop_timestamp_trigger(manager, User::Table.to_string())
            .await?;
        manager
            .drop_table(Table::drop().table(User::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
#[allow(dead_code)]
enum User {
    Table,
    Id,
    Username,
    PasswordHash,
    IsAdmin,
    CreatedAt,
}
