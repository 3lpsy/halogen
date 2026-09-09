use super::*;

impl ApiClient {
    /// GET /healthz — unauthenticated liveness probe (used by the login
    /// reachability check). Lives at the app root, outside the `/api/v1` prefix
    /// baked into `self.base`, so strip the prefix to reach it.
    pub async fn health(&self) -> Result<StatusData, ApiError> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.is_local() {
            let response = self
                .dispatch_local("GET", &format!("{}/version", self.base), None)
                .await?
                .ok_or_else(|| ApiError::Decode("missing local transport".into()))?;
            return Ok(StatusData {
                running: response.status().is_success(),
            });
        }
        let origin = self
            .base
            .strip_suffix("/api/v1")
            .unwrap_or(self.base.as_str());
        let url = format!("{origin}/healthz");
        let response = self.http.get(url).send().await?;
        self.handle_single_response(response).await
    }

    /// POST /ws-ticket — mint a short-lived ticket for the connectivity
    /// WebSocket. Authed (bearer): a browser WebSocket handshake can't carry an
    /// `Authorization` header, so the client mints this first and passes it as the
    /// `?ticket=` query param on the `/ws` upgrade. Re-minted on every (re)connect.
    pub async fn mint_ws_ticket(&self) -> Result<WsTicketData, ApiError> {
        let url = format!("{}{}", self.base, "/ws-ticket");
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// The `ws://`/`wss://` URL for the connectivity socket, derived from the base
    /// (scheme `http→ws` / `https→wss`, path `/api/v1/ws`). The ticket is appended
    /// by the caller — kept off this value so it never lands in a log.
    pub fn ws_url(&self) -> String {
        let base = self.base.as_str();
        if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}/ws")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}/ws")
        } else {
            // No recognised scheme (shouldn't happen — `new` builds from a `Url`);
            // fall back to a ws:// best-effort rather than panicking.
            format!("ws://{base}/ws")
        }
    }

    /// POST /auth/login — authenticate and return a token.
    pub async fn login(&self, creds: LoginData) -> Result<TokenData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(creds);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/auth/login");
        let req = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_str);
        // wasm: opt the fetch into storing the `auth_media` cookie cross-origin
        // (default credentials mode is `same-origin`).
        #[cfg(target_arch = "wasm32")]
        let req = req.fetch_credentials_include();
        let response = req.send().await?;
        self.handle_single_response(response).await
    }

    /// POST /auth/refresh — refresh an existing token.
    pub async fn refresh(&self, token: TokenData) -> Result<TokenData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(token);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/auth/refresh");
        let req = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_str);
        // wasm: refresh re-sets the `auth_media` cookie — keep credentials on.
        #[cfg(target_arch = "wasm32")]
        let req = req.fetch_credentials_include();
        let response = req.send().await?;
        self.handle_single_response(response).await
    }

    /// POST /auth/logout — stateless logout.
    pub async fn logout(&self) -> Result<(), ApiError> {
        let url = format!("{}{}", self.base, "/auth/logout");
        let req = self.http.post(url);
        // wasm: needed so the server's cookie-clearing `Set-Cookie` is honoured.
        #[cfg(target_arch = "wasm32")]
        let req = req.fetch_credentials_include();
        let response = req.send().await?;
        self.handle_empty_response(response).await
    }
}
