use anyhow::{Result, anyhow};
use bcrypt::hash;
use halogen_orm::user::Entity as UserEntity;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, Statement};
use tracing::info;

pub async fn seed_admin_user(
    dbc: &DatabaseConnection,
    username: &str,
    password: Option<&str>,
) -> Result<(bool, String)> {
    // Usernames are stored lowercase everywhere (login lowercases before lookup),
    // so normalize the configured admin name too.
    let username = username.to_lowercase();
    let count = UserEntity::find().count(dbc).await?;

    if count > 0 {
        info!("Admin user(s) already exist — skipping seed");
        return Ok((false, String::new()));
    }

    let pw_buf = password
        .map(String::from)
        .unwrap_or_else(halogen_orm::user::generate_password);
    let password_hash = hash(&pw_buf, bcrypt::DEFAULT_COST)
        .map_err(|e| anyhow!("Failed to hash password: {}", e))?;

    let admin_id = i32::MAX;
    let stmt = Statement::from_sql_and_values(
        dbc.get_database_backend(),
        "INSERT INTO user (id, username, password_hash, is_admin, created_at, updated_at) VALUES (?, ?, ?, 1, datetime('now'), datetime('now'))",
        [
            admin_id.into(),
            username.as_str().into(),
            password_hash.into(),
        ],
    );

    dbc.execute_raw(stmt)
        .await
        .map_err(|e| anyhow!("Failed to insert admin user: {}", e))?;

    info!(
        "Created initial admin user '{}' — please change this password on first login",
        username
    );
    // Print only generated passwords, as promised by the CLI. Caller-supplied secrets must never reach logs.
    if password.is_none() {
        info!("Admin password: {}", pw_buf);
    }

    Ok((true, pw_buf))
}

/// Insert an explicit user ID with a bcrypt password for real UI logins, including second accounts. No existing-user
/// skip guard; callers must avoid the seeded admin's i32::MAX ID.
pub async fn seed_password_user(
    dbc: &DatabaseConnection,
    id: i32,
    username: &str,
    password: &str,
    is_admin: bool,
) -> Result<()> {
    // Usernames are stored lowercase everywhere (login lowercases before lookup).
    let username = username.to_lowercase();
    let password_hash = hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| anyhow!("Failed to hash password: {}", e))?;
    let stmt = Statement::from_sql_and_values(
        dbc.get_database_backend(),
        "INSERT INTO user (id, username, password_hash, is_admin, created_at, updated_at) VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
        [
            id.into(),
            username.as_str().into(),
            password_hash.into(),
            i32::from(is_admin).into(),
        ],
    );
    dbc.execute_raw(stmt)
        .await
        .map_err(|e| anyhow!("Failed to insert user: {}", e))?;
    Ok(())
}
