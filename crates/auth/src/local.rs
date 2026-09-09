use crate::JwtAuth;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use halogen_wire_meta::error::ApiError;
use sea_orm::{DatabaseConnection, EntityTrait};

/// Resolve the trusted local profile on every request so deletion and demotion take effect.
pub async fn local_actor(
    State((db, id)): State<(DatabaseConnection, i32)>,
    mut request: Request,
    next: Next,
) -> Response {
    let actor = match halogen_orm::user::Entity::find_by_id(id).one(&db).await {
        Ok(Some(user)) if id > 0 => JwtAuth {
            user_id: user.id.to_string(),
            username: user.username,
            is_admin: user.is_admin,
        },
        Ok(_) => {
            return ApiError::unauthenticated("Local profile no longer exists".into())
                .into_response::<()>();
        }
        Err(_) => {
            return ApiError::new("database", "panic", "Profile lookup failed".into())
                .into_response::<()>();
        }
    };
    request.extensions_mut().insert(actor);
    next.run(request).await
}
