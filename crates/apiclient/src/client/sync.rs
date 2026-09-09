use super::{ApiClient, ApiError, build_url};
use halogen_wire::{SyncChangesData, SyncChangesParams};

impl ApiClient {
    /// Fetch one actor-scoped change batch from an opaque sync cursor.
    pub async fn get_sync_changes(
        &self,
        params: SyncChangesParams,
    ) -> Result<SyncChangesData, ApiError> {
        let query = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = self
            .authed_get(build_url(&self.base, "/sync/changes", &query))
            .await?;
        self.handle_single_response(response).await
    }
}
