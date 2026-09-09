use super::*;

impl ApiClient {
    /// GET /playlists/{id}/episodes — list episodes in a playlist with pagination.
    pub async fn list_playlist_episodes(
        &self,
        playlist_id: i32,
        params: halogen_wire::DefaultListParams<EpisodeInclude>,
    ) -> Result<Page<Vec<EpisodeData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(
            &self.base,
            &format!("/playlists/{}/episodes", playlist_id),
            &qs,
        );
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }
}
