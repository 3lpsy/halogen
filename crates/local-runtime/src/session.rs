use halogen_routes::routers::{extractors::Actor, middleware::JwtAuth};
use std::sync::Arc;

pub struct LocalSession {
    pub(crate) library: Arc<crate::Library>,
    pub(crate) user: halogen_orm::user::Model,
    router: axum::Router,
}

impl LocalSession {
    pub(crate) fn new(library: Arc<crate::Library>, user: halogen_orm::user::Model) -> Self {
        let router = halogen_router::router_local(
            library.db.clone(),
            &library.config,
            library.polling.clone(),
            halogen_runtime_control::RestartHandle::new(),
            JwtAuth {
                user_id: user.id.to_string(),
                username: user.username.clone(),
                is_admin: user.is_admin,
            },
        );
        Self {
            library,
            user,
            router,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.user.is_admin
    }
    pub fn user_id(&self) -> i32 {
        self.user.id
    }
    pub fn username(&self) -> &str {
        &self.user.username
    }

    pub(crate) async fn current_actor(&self) -> Result<Actor, String> {
        use sea_orm::EntityTrait;
        let user = halogen_orm::user::Entity::find_by_id(self.user.id)
            .one(&self.library.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("local profile no longer exists")?;
        Ok(Actor {
            id: user.id,
            is_admin: user.is_admin,
        })
    }

    pub async fn invoke(&self, request: crate::ApiRequest) -> Result<crate::ApiResponse, String> {
        let _guard = self.library.activity.read().await;
        self.library.ensure_active()?;
        halogen_router::dispatch(&self.router, request).await
    }
}

impl halogen_apiclient::LocalTransport for LocalSession {
    fn media_path<'a>(&'a self, path: &'a str) -> halogen_apiclient::MediaPathFuture<'a> {
        Box::pin(LocalSession::media_path(self, path))
    }
    fn invoke(&self, request: crate::ApiRequest) -> halogen_apiclient::DispatchFuture<'_> {
        Box::pin(LocalSession::invoke(self, request))
    }
}
