use serde::{Deserialize, Serialize};

/// Scope claim value marking a **media-only** token. Such a token authenticates
/// the audio endpoint via the `auth_media` cookie and is *rejected* by the API
/// bearer middleware, so a leaked media credential can't touch the API (and the
/// API token can't stream media).
pub const MEDIA_SCOPE: &str = "media";

/// Scope claim value marking a **WebSocket ticket** — a short-lived credential minted by `POST /ws-ticket` and
/// passed as the `?ticket=` query param on the `/ws` upgrade (browsers can't set an `Authorization` header on a
/// WebSocket). Scope-locked like [`MEDIA_SCOPE`]: rejected by the API bearer middleware, and the WS handshake
/// accepts *only* this scope, so neither token type works on the other path.
pub const WS_SCOPE: &str = "ws";

/// JWT claims used in the token. `scope` differentiates credentials: `None` (default) is a normal full-access
/// API token; `Some("media")` is a media-only token (see [`MEDIA_SCOPE`]); `Some("ws")` is a WebSocket ticket
/// (see [`WS_SCOPE`]). The field is `#[serde(default)]` so older tokens without it decode as API tokens.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub exp: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl JwtClaims {
    /// A full-access API token (no scope).
    pub fn api(sub: String, exp: usize) -> Self {
        Self {
            sub,
            exp,
            scope: None,
        }
    }

    /// A media-only token (`scope = "media"`).
    pub fn media(sub: String, exp: usize) -> Self {
        Self {
            sub,
            exp,
            scope: Some(MEDIA_SCOPE.to_string()),
        }
    }

    /// A WebSocket ticket (`scope = "ws"`).
    pub fn ws(sub: String, exp: usize) -> Self {
        Self {
            sub,
            exp,
            scope: Some(WS_SCOPE.to_string()),
        }
    }

    /// True when this is a normal full-access API token (no scope). The inverse of
    /// "any scope-locked credential" — use this (not `!is_media()`) wherever only a
    /// real API token is acceptable, so a `ws` ticket isn't silently let through.
    pub fn is_api(&self) -> bool {
        self.scope.is_none()
    }

    /// True when this is a media-scoped token.
    pub fn is_media(&self) -> bool {
        self.scope.as_deref() == Some(MEDIA_SCOPE)
    }

    /// True when this is a WebSocket ticket.
    pub fn is_ws(&self) -> bool {
        self.scope.as_deref() == Some(WS_SCOPE)
    }
}

/// Authenticated user info extracted from a valid JWT.
#[derive(Debug, Clone)]
pub struct JwtAuth {
    pub user_id: String,
    pub username: String,
    pub is_admin: bool,
}

/// Auth configuration (JWT secret + expiry). Consumed by [`JwtAuthLayer`] AND
/// handed to the login/refresh/media handlers as an `Extension` — it's the same
/// two values, so one struct serves both.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub secret: String,
    pub expiry_secs: u64,
}
