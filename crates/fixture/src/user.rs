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
    // Print the password ONLY when we generated it (the CLI's "random password
    // printed if omitted" contract). A caller-supplied one must never land in
    // logs — the embedded server passes its secrets-file password here, and
    // its tracing output feeds the user-viewable, exportable device log.
    if password.is_none() {
        info!("Admin password: {}", pw_buf);
    }

    Ok((true, pw_buf))
}

/// Insert a user with a REAL (bcrypt-hashed) loginable password at an explicit
/// `id`. Unlike [`seed_admin_user`], there is NO "skip when a user already
/// exists" guard, so this can create a SECOND account for multi-account tests
/// (the e2e add-account flow logs a second user in through the UI, which needs
/// a genuine hash — `TestApp::seed_user`'s placeholder hash can't authenticate).
/// The caller supplies the id to avoid colliding with the admin's `i32::MAX`.
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
