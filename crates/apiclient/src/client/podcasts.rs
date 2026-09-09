use super::*;

impl ApiClient {
    /// GET /podcasts — list podcasts with pagination.
    pub async fn list_podcasts(
        &self,
        params: halogen_wire::DefaultListParams<PodcastInclude>,
    ) -> Result<Page<Vec<PodcastData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/podcasts", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /podcasts/{id} — always requests the `podcast_config` include so the
    /// returned podcast carries its download/poll override (the config form prefills
    /// from it, and the detail page can render the config offline).
    pub async fn get_podcast(&self, id: i32) -> Result<PodcastData, ApiError> {
        let params = halogen_wire::DefaultGetParams::<PodcastInclude> {
            id: None,
            includes: Some(vec![PodcastInclude::PodcastConfig]),
        };
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let path = format!("/podcasts/{}", id);
        let url = build_url(&self.base, &path, &qs);
        let response = self.authed_get(url).await?;
        let resp: Page<PodcastData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /podcasts
    pub async fn create_podcast(&self, data: PodcastStoreData) -> Result<PodcastData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/podcasts");
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PodcastData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /podcasts/{id}
    pub async fn update_podcast(
        &self,
        id: i32,
        data: PodcastUpdateData,
    ) -> Result<PodcastData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcasts/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<PodcastData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /podcasts/{id}
    pub async fn delete_podcast(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/podcasts/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }
}
