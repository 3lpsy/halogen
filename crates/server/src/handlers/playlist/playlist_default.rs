use halogen_wire::{DefaultPlaylistData, PlaylistData, PlaylistInclude, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::handlers::db_error;
use halogen_orm::playlist::{Column, Entity as PlaylistEntity};

/// The default ("queue") playlist with its ordered episode ids, or `None` when no
/// default exists. A targeted lookup so the client always knows its queue,
/// independent of how the playlist list is paged.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
) -> Result<DefaultPlaylistData, ValidationErrors> {
    let found = PlaylistEntity::find()
        .filter(Column::UserId.eq(user_id))
        .filter(Column::IsDefault.eq(true))
        .one(dbc)
        .await
        .map_err(db_error("fetching default playlist"))?;

    let playlist = match found {
        Some(model) => {
            let mut data = PlaylistData::from(model);
            super::attach_episode_includes(
                dbc,
                std::slice::from_mut(&mut data),
                Some(&vec![PlaylistInclude::EpisodeIds]),
            )
            .await?;
            Some(data)
        }
        None => None,
    };

    Ok(DefaultPlaylistData { playlist })
}
