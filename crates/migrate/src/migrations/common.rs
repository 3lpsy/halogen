use sea_orm_migration::prelude::*;
pub trait TableWithTimestamps {
    fn add_timestamps(&mut self) -> &mut Self;
}
impl TableWithTimestamps for TableCreateStatement {
    fn add_timestamps(&mut self) -> &mut Self {
        self.col(
            ColumnDef::new(Timestamp::CreatedAt)
                .timestamp()
                .not_null()
                .default(SimpleExpr::Custom("CURRENT_TIMESTAMP".into())),
        )
        .col(
            ColumnDef::new(Timestamp::UpdatedAt)
                .timestamp()
                .not_null()
                .default(SimpleExpr::Custom("CURRENT_TIMESTAMP".into())),
        )
    }
}

// Create an enum for the timestamp column names
#[derive(DeriveIden)]
enum Timestamp {
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
pub trait MigrationTimestampExt {
    async fn create_timestamp_trigger(
        &self,
        manager: &SchemaManager,
        table: String,
    ) -> Result<(), DbErr>;

    /// Like [`create_timestamp_trigger`](Self::create_timestamp_trigger) but
    /// matches the updated row by `rowid` instead of `id`, so it works for
    /// composite-PK or surrogate-key-less junction tables. Every table here is a
    /// rowid table (none are `WITHOUT ROWID`). Same trigger name, so
    /// `drop_timestamp_trigger` drops it.
    async fn create_rowid_timestamp_trigger(
        &self,
        manager: &SchemaManager,
        table: String,
    ) -> Result<(), DbErr>;

    async fn drop_timestamp_trigger(
        &self,
        manager: &SchemaManager,
        table: String,
    ) -> Result<(), DbErr>;
}

#[async_trait::async_trait]
impl<T> MigrationTimestampExt for T
where
    T: MigrationTrait + Sync + Send,
{
    async fn create_timestamp_trigger(
        &self,
        manager: &SchemaManager,
        table: String,
    ) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let sql = format!(
            r#"
            CREATE TRIGGER IF NOT EXISTS "{table_name}_update_timestamp"
            AFTER UPDATE ON "{table_name}"
            FOR EACH ROW
            BEGIN
                UPDATE "{table_name}" SET {update_col} = CURRENT_TIMESTAMP
                WHERE id = NEW.id AND
                        (OLD.{update_col} IS NULL OR OLD.{update_col} = NEW.{update_col});
            END;
            "#,
            table_name = table,
            update_col = Timestamp::UpdatedAt.to_string()
        );

        db.execute_unprepared(&sql).await?;

        Ok(())
    }

    async fn create_rowid_timestamp_trigger(
        &self,
        manager: &SchemaManager,
        table: String,
    ) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let sql = format!(
            r#"
            CREATE TRIGGER IF NOT EXISTS "{table_name}_update_timestamp"
            AFTER UPDATE ON "{table_name}"
            FOR EACH ROW
            BEGIN
                UPDATE "{table_name}" SET {update_col} = CURRENT_TIMESTAMP
                WHERE rowid = NEW.rowid AND
                        (OLD.{update_col} IS NULL OR OLD.{update_col} = NEW.{update_col});
            END;
            "#,
            table_name = table,
            update_col = Timestamp::UpdatedAt.to_string()
        );

        db.execute_unprepared(&sql).await?;

        Ok(())
    }

    async fn drop_timestamp_trigger(
        &self,
        manager: &SchemaManager,
        table: String,
    ) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let sql = format!("DROP TRIGGER IF EXISTS {}_update_timestamp;", table);

        db.execute_unprepared(&sql).await?;

        Ok(())
    }
}
