use axum::{Extension, Json};
use halogen_wire::{DefaultListParams, NoInclude, ResponseData, UserData};
use sea_orm::DatabaseConnection;

use crate::handlers::user::user_list;
use crate::routers::ApiError;
use crate::routers::extractors::{AdminUser, Query};

/// List users (admin-only) with pagination and ordering.
pub async fn list(
    Extension(dbc): Extension<DatabaseConnection>,
    _admin: AdminUser,
    Query(params): Query<DefaultListParams<NoInclude>>,
) -> Result<Json<ResponseData<Vec<UserData>>>, ApiError> {
    let (users, paginator) = user_list::handle(&dbc, &params).await?;
    Ok(Json(ResponseData::from_paginator(users, paginator)))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
