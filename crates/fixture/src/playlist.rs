use anyhow::{Result, anyhow};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter,
};
use tracing::info;

use halogen_orm::playlist::{ActiveModel, Column, Entity as PlaylistEntity};
use halogen_orm::user::Entity as UserEntity;

/// Ensure the primary user has a default "Queue" playlist (the `is_default`
/// queue-backing list the UI uses). Defaults are **per-user** now, so this seeds
/// one for the first user (the seeded admin). Idempotent: no-ops if there's no
/// user yet, or if that user already has a default playlist. Returns whether a new
/// one was created.
pub async fn seed_default_queue(dbc: &DatabaseConnection) -> Result<bool> {
    let Some(user) = UserEntity::find().one(dbc).await? else {
        info!("No user present — skipping default queue seed");
        return Ok(false);
    };

    let existing = PlaylistEntity::find()
        .filter(Column::IsDefault.eq(true))
        .filter(Column::UserId.eq(user.id))
        .count(dbc)
        .await?;
    if existing > 0 {
        info!("Default queue playlist already exists — skipping seed");
        return Ok(false);
    }

    let now = chrono::Utc::now();
    let playlist = ActiveModel {
        name: ActiveValue::set("Queue".to_string()),
        description: ActiveValue::set(None),
        user_id: ActiveValue::set(user.id),
        is_default: ActiveValue::set(true),
        created_at: ActiveValue::set(now),
        updated_at: ActiveValue::set(now),
        ..Default::default()
    };
    playlist
        .insert(dbc)
        .await
        .map_err(|e| anyhow!("Failed to insert default queue playlist: {e}"))?;

    info!("Created default \"Queue\" playlist for user {}", user.id);
    Ok(true)
}
