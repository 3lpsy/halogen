use crate::ApiError;
use halogen_wire_meta::api::{ApiRequest, ApiResponse};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};

pub type MediaPathFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<String>, String>> + Send + 'a>>;

pub type DispatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ApiResponse, String>> + Send + 'a>>;

/// Native clients share handlers through this boundary without binding a port.
pub trait LocalTransport: Send + Sync {
    fn invoke(&self, request: ApiRequest) -> DispatchFuture<'_>;
    fn media_path<'a>(&'a self, _path: &'a str) -> MediaPathFuture<'a> {
        Box::pin(async { Err("local media is unavailable".into()) })
    }
}

type Resolver = fn(&url::Url) -> Option<Arc<dyn LocalTransport>>;
static LOCAL_RESOLVER: OnceLock<Resolver> = OnceLock::new();

pub fn install_local_resolver(resolver: Resolver) {
    let _ = LOCAL_RESOLVER.set(resolver);
}

pub(crate) fn resolve(url: &url::Url) -> Option<Arc<dyn LocalTransport>> {
    LOCAL_RESOLVER.get().and_then(|resolve| resolve(url))
}

impl crate::ApiClient {
    pub fn local(transport: Arc<dyn LocalTransport>) -> Self {
        let mut client = Self::new(url::Url::parse("halogen-local://library").expect("static URL"));
        client.local = Some(transport);
        client
    }

    pub async fn local_media_path(&self, route: &str) -> Result<Option<String>, ApiError> {
        let transport = self
            .local
            .as_ref()
            .ok_or_else(|| ApiError::Decode("not a local client".into()))?;
        transport
            .media_path(route)
            .await
            .map_err(|message| ApiError::Server {
                status: 500,
                message,
            })
    }

    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    pub(crate) async fn dispatch_local(
        &self,
        method: &str,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Option<reqwest::Response>, ApiError> {
        let Some(transport) = &self.local else {
            if url.starts_with("halogen-local:") {
                return Err(ApiError::Server {
                    status: 503,
                    message: "Local library is unavailable".into(),
                });
            }
            return Ok(None);
        };
        let url = url::Url::parse(url).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = transport
            .invoke(ApiRequest {
                method: method.into(),
                path: url.path().into(),
                query: url.query().map(str::to_owned),
                body,
            })
            .await
            .map_err(|message| ApiError::Server {
                status: 500,
                message,
            })?;
        let response = http::Response::builder()
            .status(response.status)
            .body(response.body)
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(Some(reqwest::Response::from(response)))
    }
}
