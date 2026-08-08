use halogen_wire::{DefaultListParams, NoInclude, Paginator, UserData, ValidationErrors};
use sea_orm::EntityTrait;
use tracing::info;

use halogen_orm::user::Entity as UserEntity;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    params: &DefaultListParams<NoInclude>,
) -> Result<(Vec<UserData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    let order = params.order.clone().unwrap_or_default();

    let (users, paginator) =
        halogen_orm::common::paginate(dbc, UserEntity::find(), &pagination, &order).await?;
    let responses: Vec<UserData> = users.into_iter().map(UserData::from).collect();
    info!(
        "Fetched {} users (page {}, size {})",
        responses.len(),
        pagination.page,
        pagination.size
    );
    Ok((responses, paginator))
}
