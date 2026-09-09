use halogen_wire::{DefaultListParams, Paginator, PlaylistData, PlaylistInclude, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::info;

use halogen_orm::playlist::{Column, Entity as PlaylistEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    params: &DefaultListParams<PlaylistInclude>,
) -> Result<(Vec<PlaylistData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    let order = params.order.clone().unwrap_or_default();

    // Scope to the caller: a user only sees the playlists they own. An optional
    // `search` filter narrows by name (LIKE %s%) so the client's lazy list can
    // page server-side matches instead of only filtering the cached pool.
    let mut query = PlaylistEntity::find().filter(Column::UserId.eq(user_id));
    if let Some(search) = params.filter.as_ref().and_then(|f| f.search.as_ref())
        && !search.is_empty()
    {
        query = query.filter(Column::Name.contains(search.as_str()));
    }

    let (playlists, paginator) =
        halogen_orm::common::paginate(dbc, query, &pagination, &order).await?;

    let mut data: Vec<PlaylistData> = playlists.into_iter().map(|p| p.into()).collect();
    super::attach_episode_includes(dbc, &mut data, params.includes.as_ref()).await?;

    info!("Fetched {} playlists", data.len());
    Ok((data, paginator))
}
