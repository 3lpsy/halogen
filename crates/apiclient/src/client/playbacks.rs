use super::*;

impl ApiClient {
    /// GET /playbacks — list playbacks with pagination.
    pub async fn list_playbacks(
        &self,
        params: PlaybackListParams,
    ) -> Result<Page<Vec<PlaybackData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/playbacks", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// POST /playbacks — upsert a playback.
    pub async fn upsert_playback(&self, data: PlaybackStoreData) -> Result<PlaybackData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/playbacks");
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PlaybackData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /playbacks/{id}
    pub async fn delete_playback(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/playbacks/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }
}
