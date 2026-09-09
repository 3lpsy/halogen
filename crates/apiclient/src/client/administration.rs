use super::*;

impl ApiClient {
    /// GET /status — whether the background polling service is running.
    pub async fn poll_status(&self) -> Result<PollingStatusData, ApiError> {
        let url = format!("{}/status", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/poll — trigger a one-off feed sync immediately. **Admin only.**
    pub async fn poll_now(&self) -> Result<PollingOperationData, ApiError> {
        let url = format!("{}/admin/poll", self.base);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/poll-job — start an on-demand poll job and get its id back
    /// immediately. `podcast_id` scopes the run to one feed (`None` = all). Poll
    /// [`Self::get_poll_job`] for progress. **Admin only.**
    pub async fn start_poll_job(
        &self,
        podcast_id: Option<i32>,
    ) -> Result<PollJobStartData, ApiError> {
        let qs = match podcast_id {
            Some(id) => format!("podcast_id={id}"),
            None => String::new(),
        };
        let url = build_url(&self.base, "/admin/poll-job", &qs);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/poll-job/{id} — current snapshot of a poll job. **Admin only.**
    pub async fn get_poll_job(&self, id: u64) -> Result<PollJobData, ApiError> {
        let url = format!("{}/admin/poll-job/{}", self.base, id);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/poll-jobs — recent poll jobs, newest first (capped). **Admin only.**
    pub async fn list_poll_jobs(&self) -> Result<Vec<PollJobData>, ApiError> {
        let url = format!("{}/admin/poll-jobs", self.base);
        let response = self.authed_get(url).await?;
        let resp: Page<Vec<PollJobData>> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// POST /admin/start — start the background polling service. **Admin only.**
    pub async fn start_polling(&self) -> Result<PollingOperationData, ApiError> {
        let url = format!("{}/admin/start", self.base);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/stop — stop the background polling service. **Admin only.**
    pub async fn stop_polling(&self) -> Result<PollingOperationData, ApiError> {
        let url = format!("{}/admin/stop", self.base);
        let response = self.authed_post(url, String::new()).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/config — the reconciled runtime config, minus secrets. **Admin only.**
    ///
    /// Returns a single sanitised [`ConfigData`] (not paginated), so it uses the
    /// single-object response handling like [`Self::health`].
    pub async fn get_config(&self) -> Result<ConfigData, ApiError> {
        let url = format!("{}/admin/config", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/config-overrides — the persisted overrides allowlist (only keys that
    /// are actually set). **Admin only.** Empty when none. Prepopulates the editor.
    pub async fn get_config_overrides(&self) -> Result<ConfigOverridesData, ApiError> {
        let url = format!("{}/admin/config-overrides", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// POST /admin/config-overrides — **replace** the overrides with `data` verbatim.
    /// **Admin only.** Any allowlisted key omitted from `data` is deleted. Does
    /// not restart — apply via [`Self::restart_server`]. Returns the written set.
    pub async fn set_config_overrides(
        &self,
        data: ConfigOverridesData,
    ) -> Result<ConfigOverridesData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/admin/config-overrides", self.base);
        let response = self.authed_post(url, body_str).await?;
        self.handle_single_response(response).await
    }

    /// DELETE /admin/config-overrides — clear ALL overrides. **Admin only.** Does not
    /// restart.
    pub async fn delete_config_overrides(&self) -> Result<(), ApiError> {
        let url = format!("{}/admin/config-overrides", self.base);
        let response = self.authed_delete(url).await?;
        self.handle_empty_response(response).await
    }

    /// POST /admin/server/restart — request a graceful re-exec so written overrides take
    /// effect. **Admin only.** The connection drops while the process restarts.
    pub async fn restart_server(&self) -> Result<(), ApiError> {
        let url = format!("{}/admin/server/restart", self.base);
        let response = self.authed_post(url, String::new()).await?;
        // The handler echoes a small status object; we only care about success.
        let _: PollingOperationData = self.handle_single_response(response).await?;
        Ok(())
    }

    /// POST /admin/users — create a user. **Admin only.** Powers the embedded
    /// server's add-account flow (the app generates + stores the password for
    /// silent login) and ordinary provisioning.
    pub async fn create_user(&self, data: UserStoreData) -> Result<UserData, ApiError> {
        let body = halogen_wire::RequestData::<_, ()>::from_data(data);
        let body_str = serde_json::to_string(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = format!("{}/admin/users", self.base);
        let response = self.authed_post(url, body_str).await?;
        let resp: Page<UserData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// GET /admin/db/export — the gzipped, scrubbed database export (no
    /// password hashes, nothing marked downloaded, no operational history).
    /// **Admin only.** Returns the raw `.db.gz` bytes for the caller to save.
    pub async fn export_db(&self) -> Result<Vec<u8>, ApiError> {
        let url = format!("{}/admin/db/export", self.base);
        let response = self.authed_get(url).await?;
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await.unwrap_or_default();
            // Either arm of `parse_error_body` is an error to surface.
            return match parse_error_body(status.as_u16(), &bytes) {
                Ok(e) | Err(e) => Err(e),
            };
        }
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// POST /admin/db/import — merge an export (gzipped or raw SQLite) into
    /// the server's database. **Admin only.** Returns the per-entity summary
    /// (see the DTO for the merge rules).
    pub async fn import_db(&self, bytes: Vec<u8>) -> Result<DbImportSummaryData, ApiError> {
        let url = format!("{}/admin/db/import", self.base);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(response) = self
            .dispatch_local("POST", &url, Some(bytes.clone()))
            .await?
        {
            return self.handle_single_response(response).await;
        }
        let mut builder = self.http.post(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(ApiError::from)?;
        self.handle_single_response(response).await
    }

    /// GET /admin/server-errors — the persisted failure histories (RSS sync per
    /// podcast, media download per episode), newest first. **Admin only.**
    pub async fn get_server_errors(&self) -> Result<ServerErrorsData, ApiError> {
        let url = format!("{}/admin/server-errors", self.base);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }

    /// GET /admin/server-logs — tail of the server's log file. **Admin only.**
    /// `lines` caps how many trailing lines come back (server default/caps apply).
    pub async fn get_server_logs(&self, lines: Option<usize>) -> Result<ServerLogsData, ApiError> {
        let qs = match lines {
            Some(n) => format!("lines={n}"),
            None => String::new(),
        };
        let url = build_url(&self.base, "/admin/server-logs", &qs);
        let response = self.authed_get(url).await?;
        self.handle_single_response(response).await
    }
}
