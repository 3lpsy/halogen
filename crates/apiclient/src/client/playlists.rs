use super::*;

impl ApiClient {
    /// GET /playlists — list playlists with pagination.
    pub async fn list_playlists(
        &self,
        params: halogen_wire::DefaultListParams<PlaylistInclude>,
    ) -> Result<Page<Vec<PlaylistData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/playlists", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /playlists/{id}
    pub async fn get_playlist(&self, id: i32) -> Result<PlaylistData, ApiError> {
        self.get_playlist_including(id, &[]).await
    }

    /// Fetch authoritative membership when reconciling a rejected local mutation.
    pub async fn get_playlist_including(
        &self,
        id: i32,
        includes: &[PlaylistInclude],
    ) -> Result<PlaylistData, ApiError> {
        let params = halogen_wire::DefaultGetParams::<PlaylistInclude> {
            id: None,
            includes: Some(includes.to_vec()),
        };
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, &format!("/playlists/{id}"), &qs);
        let response = self.authed_get(url).await?;
        let resp: Page<PlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// GET /playlists/default — the user's queue (default playlist) with its
    /// ordered episode ids, or `None` when no default exists. Decoupled from the
    /// paged playlist list so the client always knows its queue.
    pub async fn get_default_playlist(&self) -> Result<Option<PlaylistData>, ApiError> {
        let url = format!("{}/playlists/default", self.base);
        let response = self.authed_get(url).await?;
        let resp: Page<DefaultPlaylistData> = self.handle_response(response).await?;
        Ok(resp.data.playlist)
    }

    /// POST /playlists
    pub async fn create_playlist(&self, data: PlaylistStoreData) -> Result<PlaylistData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}{}", self.base, "/playlists");
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<PlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /playlists/{id}
    pub async fn update_playlist(
        &self,
        id: i32,
        data: PlaylistUpdateData,
    ) -> Result<PlaylistData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<PlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /playlists/{id}
    pub async fn delete_playlist(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/playlists/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{playlist_id}/episodes/{episode_id}
    ///
    /// `position` is the insert index: `Some(0)` = front, `None` = append (default).
    pub async fn add_episode(
        &self,
        playlist_id: i32,
        episode_id: i32,
        position: Option<i32>,
    ) -> Result<EpisodePlaylistData, ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistStoreData { position });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!(
            "{}/playlists/{}/episodes/{}",
            self.base, playlist_id, episode_id
        );
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<EpisodePlaylistData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// DELETE /playlists/{playlist_id}/episodes/{episode_id}
    pub async fn remove_episode(&self, playlist_id: i32, episode_id: i32) -> Result<(), ApiError> {
        let url = format!(
            "{}/playlists/{}/episodes/{}",
            self.base, playlist_id, episode_id
        );
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{playlist_id}/episodes/bulk — add many episodes to one
    /// playlist in a single request (append). The server owner-guards the playlist
    /// and filters out ids the caller isn't authorized for; lenient per id.
    pub async fn add_episodes_bulk(
        &self,
        playlist_id: i32,
        episode_ids: Vec<i32>,
    ) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistBulkData { episode_ids });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/episodes/bulk", self.base, playlist_id);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /playlists/{playlist_id}/episodes/bulk — remove many episodes from one
    /// playlist in a single request (non-members filtered/skipped server-side).
    pub async fn remove_episodes_bulk(
        &self,
        playlist_id: i32,
        episode_ids: Vec<i32>,
    ) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistBulkData { episode_ids });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/episodes/bulk", self.base, playlist_id);
        let response = self.authed_delete_with_body(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// POST /playlists/{playlist_id}/episodes/{episode_id}/move — reorder an
    /// episode to target index `to`; the server rewrites positions 0..n.
    pub async fn move_episode(
        &self,
        playlist_id: i32,
        episode_id: i32,
        to: i32,
    ) -> Result<(), ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(EpisodePlaylistMoveData { to });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!(
            "{}/playlists/{}/episodes/{}/move",
            self.base, playlist_id, episode_id
        );
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{id}/move — reorder a playlist to target index `to`; the
    /// server rewrites every playlist's `position` 0..n.
    pub async fn move_playlist(&self, playlist_id: i32, to: i32) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::PlaylistMoveData { to });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/move", self.base, playlist_id);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// POST /playlists/{id}/reorder-by — smart-reorder a playlist's episodes by
    /// `field`/`direction`, baking the order into the `Custom` (position) sequence.
    pub async fn reorder_playlist(
        &self,
        playlist_id: i32,
        field: halogen_wire::PlaylistReorderField,
        direction: halogen_wire::OrderDirection,
    ) -> Result<(), ApiError> {
        let body =
            halogen_wire::RequestData::<_, ()>::from_data(halogen_wire::PlaylistReorderData {
                field,
                direction,
            });
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/playlists/{}/reorder-by", self.base, playlist_id);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }

    /// GET /episodes/{id}/playlists — the caller's playlists that contain this
    /// episode (picker pre-selection).
    pub async fn list_episode_playlists(
        &self,
        episode_id: i32,
        params: halogen_wire::DefaultListParams<PlaylistInclude>,
    ) -> Result<Page<Vec<PlaylistData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(
            &self.base,
            &format!("/episodes/{}/playlists", episode_id),
            &qs,
        );
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }
}
