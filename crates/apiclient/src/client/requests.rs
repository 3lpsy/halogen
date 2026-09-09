use super::*;

impl ApiClient {
    pub(super) async fn authed_get(&self, url: String) -> Result<reqwest::Response, ApiError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(response) = self.dispatch_local("GET", &url, None).await? {
            return Ok(response);
        }
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder.send().await.map_err(ApiError::from)
    }

    pub(super) async fn authed_post(
        &self,
        url: String,
        body: String,
    ) -> Result<reqwest::Response, ApiError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(response) = self
            .dispatch_local("POST", &url, Some(body.as_bytes().to_vec()))
            .await?
        {
            return Ok(response);
        }
        let mut builder = self.http.post(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ApiError::from)
    }

    pub(super) async fn authed_put(
        &self,
        url: String,
        body: String,
    ) -> Result<reqwest::Response, ApiError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(response) = self
            .dispatch_local("PUT", &url, Some(body.as_bytes().to_vec()))
            .await?
        {
            return Ok(response);
        }
        let mut builder = self.http.put(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ApiError::from)
    }

    pub(super) async fn authed_delete(&self, url: String) -> Result<reqwest::Response, ApiError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(response) = self.dispatch_local("DELETE", &url, None).await? {
            return Ok(response);
        }
        let mut builder = self.http.delete(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder.send().await.map_err(ApiError::from)
    }

    /// DELETE with a JSON body — needed by the bulk-remove endpoint (the plain
    /// `authed_delete` sends no body). Axum's `Body<T>` extractor reads the body on
    /// DELETE just like POST.
    pub(super) async fn authed_delete_with_body(
        &self,
        url: String,
        body: String,
    ) -> Result<reqwest::Response, ApiError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(response) = self
            .dispatch_local("DELETE", &url, Some(body.as_bytes().to_vec()))
            .await?
        {
            return Ok(response);
        }
        let mut builder = self.http.delete(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ApiError::from)
    }
}
