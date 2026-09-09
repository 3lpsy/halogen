use crate::db_error;
use halogen_orm::playlist;
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, NotSet, QueryFilter, Set,
};
use std::collections::HashMap;

pub(super) async fn merge(
    txn: &DatabaseTransaction,
    src_playlists: &[playlist::Model],
    user_map: &HashMap<i32, i32>,
    summary: &mut DbImportSummaryData,
) -> Result<HashMap<i32, i32>, ValidationErrors> {
    let mut playlist_map: HashMap<i32, i32> = HashMap::new();
    // Next free per-user playlist position, computed lazily.
    let mut next_position: HashMap<i32, i32> = HashMap::new();
    for pl in src_playlists {
        let Some(&uid) = user_map.get(&pl.user_id) else {
            continue;
        };
        let existing = if pl.is_default {
            playlist::Entity::find()
                .filter(playlist::Column::UserId.eq(uid))
                .filter(playlist::Column::IsDefault.eq(true))
                .one(txn)
                .await
                .map_err(db_error("matching an imported default playlist"))?
        } else {
            playlist::Entity::find()
                .filter(playlist::Column::UserId.eq(uid))
                .filter(playlist::Column::IsDefault.eq(false))
                .filter(playlist::Column::Name.eq(pl.name.clone()))
                .one(txn)
                .await
                .map_err(db_error("matching an imported playlist"))?
        };
        match existing {
            Some(t) => {
                playlist_map.insert(pl.id, t.id);
                summary.playlists_merged += 1;
            }
            None => {
                let pos = match next_position.get(&uid) {
                    Some(p) => *p,
                    None => {
                        let max = playlist::Entity::find()
                            .filter(playlist::Column::UserId.eq(uid))
                            .all(txn)
                            .await
                            .map_err(db_error("reading target playlists"))?
                            .into_iter()
                            .map(|p| p.position)
                            .max()
                            .unwrap_or(-1);
                        max + 1
                    }
                };
                next_position.insert(uid, pos + 1);
                let created = playlist::ActiveModel {
                    id: NotSet,
                    name: Set(pl.name.clone()),
                    description: Set(pl.description.clone()),
                    user_id: Set(uid),
                    // A brand-new user keeps their imported default; a user who
                    // already had one can't grow a second (matched above).
                    is_default: Set(pl.is_default),
                    position: Set(pos),
                    on_remove_delete_file_server: Set(pl.on_remove_delete_file_server),
                    on_remove_delete_file_client: Set(pl.on_remove_delete_file_client),
                    created_at: Set(pl.created_at),
                    updated_at: Set(pl.updated_at),
                }
                .insert(txn)
                .await
                .map_err(db_error("creating an imported playlist"))?;
                playlist_map.insert(pl.id, created.id);
                summary.playlists_created += 1;
            }
        }
    }

    Ok(playlist_map)
}
