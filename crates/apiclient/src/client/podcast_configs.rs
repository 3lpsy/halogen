use super::*;

impl ApiClient {
    /// GET /podcast-configs/{id}
    pub async fn get_podcast_config(&self, id: i32) -> Result<PodcastConfigData, ApiError> {
        let url = format!("{}/podcast-configs/{}", self.base, id);
        let response = self.authed_get(url).await?;
        let resp: Page<PodcastConfigData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /podcast-configs/{id}
    pub async fn update_podcast_config(
        &self,
        id: i32,
        data: PodcastConfigUpdateData,
    ) -> Result<PodcastConfigData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcast-configs/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<PodcastConfigData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /podcasts/{id}/config — create a config AND link it to the podcast in
    /// one atomic call. This is the only way to create a config (there is no
    /// standalone, unlinked create). Returns the new config.
    pub async fn create_podcast_config_for(
        &self,
        podcast_id: i32,
        data: PodcastConfigStoreData,
    ) -> Result<PodcastConfigData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcasts/{}/config", self.base, podcast_id);
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PodcastConfigData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /podcasts/{id}/config — unlink + delete the podcast's config (revert
    /// to global defaults), atomically.
    pub async fn remove_podcast_config_for(&self, podcast_id: i32) -> Result<(), ApiError> {
        let url = format!("{}/podcasts/{}/config", self.base, podcast_id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }
}
