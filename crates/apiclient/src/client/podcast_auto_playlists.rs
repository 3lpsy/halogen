use super::*;

impl ApiClient {
    /// GET /podcasts/{id}/auto-playlists — the playlists this podcast auto-adds
    /// new episodes to. Stale links (playlist since deleted) are filtered server-
    /// side, so every returned id is a live playlist.
    pub async fn get_podcast_auto_playlists(
        &self,
        podcast_id: i32,
    ) -> Result<Vec<PodcastAutoPlaylistData>, ApiError> {
        let url = format!("{}/podcasts/{}/auto-playlists", self.base, podcast_id);
        let response = self.authed_get(url).await?;
        let resp: Page<Vec<PodcastAutoPlaylistData>> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /podcasts/{id}/auto-playlists — replace the podcast's full set of auto-add playlists with
    /// `playlist_ids` (idempotent; unknown ids are dropped server-side). `add_to_start` is the podcast's
    /// insert-position override, stamped on every link: `Some(true)` = start of the playlists, `Some(false)` =
    /// end, `None` = follow the server-wide default. Returns the resulting set.
    pub async fn set_podcast_auto_playlists(
        &self,
        podcast_id: i32,
        playlist_ids: Vec<i32>,
        add_to_start: Option<bool>,
    ) -> Result<Vec<PodcastAutoPlaylistData>, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(PodcastAutoPlaylistSetData {
            playlist_ids,
            add_to_start,
        });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/podcasts/{}/auto-playlists", self.base, podcast_id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<Vec<PodcastAutoPlaylistData>> = self.handle_response(response).await?;
        Ok(resp.data)
    }
}
