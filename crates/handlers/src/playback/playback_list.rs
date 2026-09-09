use halogen_wire::{Paginator, PlaybackData, PlaybackListParams, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use tracing::info;

use halogen_orm::playback::{Column, Entity as PlaybackEntity};

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    params: &PlaybackListParams,
) -> Result<(Vec<PlaybackData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    let order = params.order.clone().unwrap_or_default();

    // Always scope to the authenticated user; the client cannot list another
    // user's playbacks.
    let mut query = PlaybackEntity::find().filter(Column::UserId.eq(user_id));
    if let Some(episode_id) = &params.episode_id {
        query = query.filter(Column::EpisodeId.eq(*episode_id));
    }

    let (playbacks, paginator) =
        halogen_orm::common::paginate(dbc, query, &pagination, &order).await?;

    let data: Vec<PlaybackData> = playbacks.into_iter().map(|p| p.into()).collect();

    info!("Fetched {} playbacks", data.len());
    Ok((data, paginator))
}
