use chrono::{DateTime, Utc};
use rand::Rng;
use sea_orm::entity::prelude::*;
use sea_orm::{QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true)]
    pub id: i32,
    #[sea_orm(unique)]
    pub username: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ActiveModelBehavior for ActiveModel {}

/// Sort keys accepted by the user list endpoint (unknown → primary key).
impl crate::common::Sortable for Entity {
    fn order_column(order_by: &str) -> Column {
        match order_by {
            "username" => Column::Username,
            "created_at" => Column::CreatedAt,
            "updated_at" => Column::UpdatedAt,
            _ => Column::Id,
        }
    }
}

/// Ids at or above this are sentinel rows (the seeded initial admin at
/// `i32::MAX`, the dev fixture's second user at `i32::MAX - 1`, test rows in
/// the same region) and never participate in allocation.
pub const SENTINEL_ID_FLOOR: i32 = i32::MAX - 1024;

/// Next id for a NEW user row — always allocated explicitly, never left to
/// sqlite's implicit `max(rowid)+1`: the seeded initial admin sits at the
/// fixed `i32::MAX` sentinel, so the implicit successor overflows `i32` on the
/// very first user created after the seed. Counts only ids below the whole
/// sentinel REGION — filtering just `< i32::MAX` handed out `i32::MAX` itself
/// whenever a second sentinel row sat at `MAX - 1` (the dev seed), colliding
/// with the admin. Callers race-guard via the unique primary key (a
/// concurrent create surfaces as a conflict, not a corruption).
pub async fn next_available_id<C: sea_orm::ConnectionTrait>(
    conn: &C,
) -> Result<i32, sea_orm::DbErr> {
    let max = Entity::find()
        .filter(Column::Id.lt(SENTINEL_ID_FLOOR))
        .order_by_desc(Column::Id)
        .one(conn)
        .await?
        .map(|u| u.id)
        .unwrap_or(0);
    Ok(max + 1)
}

pub fn generate_password() -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%";
    let mut rng = rand::thread_rng();
    let password: String = (0..halogen_utils::constants::WT_PASSWORD_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();
    password
}
