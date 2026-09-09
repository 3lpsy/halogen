use super::*;

impl ApiClient {
    /// Fetch an allowlisted API media path with bearer auth and optional Range for the native webview bridge, whose
    /// img/audio requests cannot attach credentials. Preserve non-2xx responses for forwarding; only transport/build
    /// failures error. The caller validates the path.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch_media_raw(
        &self,
        path: &str,
        range: Option<&str>,
    ) -> Result<crate::audio::RawMediaResponse, ApiError> {
        if let Some(transport) = &self.local {
            return crate::local_media::fetch(transport.as_ref(), path, range).await;
        }
        let url = format!("{}/{path}", self.base);
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        if let Some(range) = range {
            builder = builder.header(reqwest::header::RANGE, range);
        }
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let text_header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let content_type = text_header(reqwest::header::CONTENT_TYPE);
        let content_range = text_header(reqwest::header::CONTENT_RANGE);
        let cache_control = text_header(reqwest::header::CACHE_CONTROL);
        let etag = text_header(reqwest::header::ETAG);
        let bytes = response.bytes().await?.to_vec();
        Ok(crate::audio::RawMediaResponse {
            status,
            content_type,
            content_range,
            cache_control,
            etag,
            bytes,
        })
    }

    /// POST /episodes/{id}/download — trigger a server-side download of the
    /// episode's audio. Returns once accepted (202); the server fetches in the
    /// background and the new `download_status` shows up on the next list/get.
    pub async fn trigger_download(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/episodes/{}/download", self.base, id);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /episodes/{id}/download — remove the server's downloaded copy and
    /// reset the episode to `NotDownloaded`. The status flip shows up on the next
    /// list/get pull.
    pub async fn remove_server_download(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/episodes/{}/download", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await
    }

    /// POST /episodes/download/bulk — trigger a server-side download for many
    /// episodes in one request. The server filters out ids the caller isn't
    /// authorized for; returns 202 (background fetches), like the single route.
    pub async fn trigger_download_bulk(&self, episode_ids: Vec<i32>) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::EpisodeBulkActionData {
                episode_ids,
            });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/episodes/download/bulk", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /episodes/download/bulk — remove the server's downloaded copy for
    /// many episodes in one request (unauthorized ids filtered out server-side).
    pub async fn remove_server_download_bulk(&self, episode_ids: Vec<i32>) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::EpisodeBulkActionData {
                episode_ids,
            });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/episodes/download/bulk", self.base);
        let response = self.authed_delete_with_body(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// Snapshot an in-flight server download. A 404 becomes `Ok(None)` both before start and after completion; callers
    /// use prior progress and durable `episode.download_status` to distinguish them.
    pub async fn get_download_progress(
        &self,
        id: i32,
    ) -> Result<Option<halogen_wire::DownloadProgressData>, ApiError> {
        let url = format!("{}/episodes/{}/download-progress", self.base, id);
        let response = self.authed_get(url).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp: Page<halogen_wire::DownloadProgressData> = self.handle_response(response).await?;
        Ok(Some(resp.data))
    }

    /// PUT /episodes/{id}
    pub async fn update_episode(
        &self,
        id: i32,
        data: EpisodeUpdateData,
    ) -> Result<EpisodeData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/episodes/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<EpisodeData> = self.handle_response(response).await?;
        Ok(resp.data)
    }
}
