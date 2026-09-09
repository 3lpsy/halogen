use super::upload::bad_request;
use crate::db_error;
use halogen_orm::user;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_users: &[user::Model],
    summary: &mut DbImportSummaryData,
) -> Result<HashMap<i32, i32>, ValidationErrors> {
    let mut user_map: HashMap<i32, i32> = HashMap::new();
    for su in src_users {
        let uname = su.username.to_lowercase();
        let existing = user::Entity::find()
            .filter(user::Column::Username.eq(uname.clone()))
            .one(txn)
            .await
            .map_err(db_error("matching an imported user"))?;
        match existing {
            Some(t) => {
                user_map.insert(su.id, t.id);
                summary.users_merged += 1;
            }
            None => {
                // Stripped exports carry empty hashes — provision a random
                // password (never returned; an embedded host re-keys its own
                // silent-login secrets from `created_usernames`).
                let hash = if su.password_hash.is_empty() {
                    bcrypt::hash(halogen_orm::user::generate_password(), bcrypt::DEFAULT_COST)
                        .map_err(|e| bad_request(format!("Failed to hash a password: {e}")))?
                } else {
                    su.password_hash.clone()
                };
                // Explicit id: the admin sentinel at i32::MAX breaks sqlite's
                // implicit successor (see `next_available_id`).
                let id = halogen_orm::user::next_available_id(txn)
                    .await
                    .map_err(db_error("allocating an imported user id"))?;
                let created = user::ActiveModel {
                    id: Set(id),
                    username: Set(uname.clone()),
                    password_hash: Set(hash),
                    is_admin: Set(su.is_admin),
                    created_at: Set(su.created_at),
                    updated_at: Set(su.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported user"))?;
                user_map.insert(su.id, created.id);
                summary.users_created += 1;
                summary.created_usernames.push(uname);
            }
        }
    }

    Ok(user_map)
}
