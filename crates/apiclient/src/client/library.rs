use super::*;

impl ApiClient {
    pub async fn discover_podcast_page(
        &self,
        params: halogen_wire::DiscoverPageParams,
    ) -> Result<halogen_wire::DiscoverPodcastPageData, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = self
            .authed_get(build_url(&self.base, "/discover/search/page", &qs))
            .await?;
        self.handle_single_response(response).await
    }

    pub async fn discover_episode_page(
        &self,
        params: halogen_wire::DiscoverPageParams,
    ) -> Result<halogen_wire::DiscoverEpisodePageData, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = self
            .authed_get(build_url(&self.base, "/discover/episodes/search/page", &qs))
            .await?;
        self.handle_single_response(response).await
    }

    pub async fn discover_episode_search(
        &self,
        params: DiscoverSearchParams,
    ) -> Result<halogen_wire::DiscoverEpisodeSearchData, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = self
            .authed_get(build_url(&self.base, "/discover/episodes/search", &qs))
            .await?;
        self.handle_single_response(response).await
    }

    pub async fn discover_podcast(
        &self,
        params: halogen_wire::DiscoverPodcastParams,
    ) -> Result<halogen_wire::DiscoverPodcastData, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = self
            .authed_get(build_url(&self.base, "/discover/podcast", &qs))
            .await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/opml/import — import podcasts from OPML XML. **Admin only.**
    pub async fn import_opml(
        &self,
        data: OpmlImportData,
    ) -> Result<OpmlImportResultData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/admin/opml/import", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/opml/export — the current subscriptions as OPML XML. **Admin only.**
    pub async fn export_opml(&self) -> Result<OpmlExportData, ApiError> {
        let url = format!("{}/admin/opml/export", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /discover/search?q=...&providers[]=itunes... — federated provider search. Online-only. Returns a
    /// single [`DiscoverSearchData`] (never paginated) carrying merged results plus a per-provider error list,
    /// so a provider failing yields partial results rather than an error. `serde_qs` urlencodes `q` and the
    /// `providers[]` filter.
    pub async fn discover_search(
        &self,
        params: DiscoverSearchParams,
    ) -> Result<DiscoverSearchData, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/discover/search", &qs);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /discover/providers — which providers exist and are currently available
    /// (so the UI knows which toggle chips to render).
    pub async fn discover_providers(&self) -> Result<DiscoverProvidersData, ApiError> {
        let url = format!("{}/discover/providers", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /users/{id}
    pub async fn get_user(&self, id: i32) -> Result<UserData, ApiError> {
        let url = format!("{}/users/{}", self.base, id);
        let response = self.authed_get(url).await?;
        let resp: Page<UserData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// PUT /users/{id}
    pub async fn update_user(&self, id: i32, data: UserUpdateData) -> Result<UserData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/users/{}", self.base, id);
        let response = self.authed_put(url, body_str).await?;
        let resp: Page<UserData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /auth/password — change the authenticated user's own password. The
    /// account is taken from the bearer token server-side (never the body), so this
    /// only ever changes the caller's own password. Online-only (no outbox).
    pub async fn change_password(&self, data: PasswordChangeData) -> Result<(), ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/auth/password", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_empty_response(response).await
    }

    /// DELETE /admin/users/{id} — remove a user. **Admin only.**
    pub async fn delete_user(&self, id: i32) -> Result<(), ApiError> {
        let url = format!("{}/admin/users/{}", self.base, id);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await?;
        Ok(())
    }
}
