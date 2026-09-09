use super::{SyncQueue, error};
use crate::{CoreError, LocalCore};
use halogen_apiclient::ApiClient;
use halogen_sync::PullOutcome;
use halogen_sync_store::{LocalStore, NativeLocalStore};
use std::sync::Arc;

#[uniffi::export(async_runtime = "tokio")]
impl SyncQueue {
    /// Recover the canonical cache if a host stopped after the delta cursor committed.
    pub async fn cached_snapshot(&self) -> Result<Option<String>, CoreError> {
        self.snapshot(None, None).await
    }

    pub async fn pull_remote(
        &self,
        base_url: String,
        token: String,
        projected_cursor: Option<String>,
    ) -> Result<Option<String>, CoreError> {
        self.snapshot(
            Some(super::remote_client(&base_url, token)?),
            projected_cursor,
        )
        .await
    }

    pub async fn pull_local(
        &self,
        core: Arc<LocalCore>,
        projected_cursor: Option<String>,
    ) -> Result<Option<String>, CoreError> {
        self.snapshot(
            Some(ApiClient::local(core.session.clone())),
            projected_cursor,
        )
        .await
    }
}

impl SyncQueue {
    async fn snapshot(
        &self,
        api: Option<ApiClient>,
        projected_cursor: Option<String>,
    ) -> Result<Option<String>, CoreError> {
        let path = self.path.clone();
        let handle = tokio::runtime::Handle::current();
        let guard = self.gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let store = NativeLocalStore::open(path).map_err(error)?;
            handle.block_on(async {
                if let Some(api) = api {
                    if matches!(
                        halogen_sync::pull_changes(&store, &api)
                            .await
                            .map_err(error)?,
                        PullOutcome::Deferred
                    ) {
                        return Ok(None);
                    }
                    if projected_cursor == store.sync_cursor().await.map_err(error)? {
                        return Ok(None);
                    }
                }
                halogen_sync::cached_snapshot(&store)
                    .await
                    .map_err(error)?
                    .map(|snapshot| serde_json::to_string(&snapshot).map_err(error))
                    .transpose()
            })
        })
        .await
        .map_err(error)?
    }
}
