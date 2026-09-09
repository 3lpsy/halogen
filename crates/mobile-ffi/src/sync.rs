mod pull;

use crate::{CoreError, LocalCore};
use halogen_apiclient::ApiClient;
use halogen_sync::{PushOutcome, push_entry};
use halogen_sync_store::{LocalStore, NativeLocalStore};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tokio::sync::Mutex;

#[derive(uniffi::Object)]
pub struct SyncQueue {
    path: PathBuf,
    gate: Arc<Mutex<()>>,
}

#[derive(uniffi::Record)]
pub struct QueuedOperation {
    pub id: String,
    pub operation_json: String,
}

#[derive(Default, uniffi::Record)]
pub struct SyncDrainReport {
    pub applied: u32,
    pub applied_ids: Vec<String>,
    pub quarantined_ids: Vec<String>,
    pub quarantined: u32,
    pub pending: u32,
    pub auth_paused: bool,
    pub last_error: Option<String>,
}

fn error(error: impl std::fmt::Display) -> CoreError {
    CoreError::Server {
        msg: error.to_string(),
    }
}

// Handles for one queue share a gate; unrelated profiles can continue independently.
fn queue_lock(path: &std::path::Path) -> Arc<Mutex<()>> {
    use std::{collections::HashMap, sync::Weak};
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.into(), Arc::downgrade(&lock));
    lock
}

#[uniffi::export]
pub fn open_sync_queue(db_path: String) -> Result<Arc<SyncQueue>, CoreError> {
    let path = PathBuf::from(db_path);
    if !path.is_absolute() {
        return Err(error("sync database path must be absolute"));
    }
    NativeLocalStore::open(path.clone()).map_err(error)?;
    let path = path.canonicalize().map_err(error)?;
    let gate = queue_lock(&path);
    Ok(Arc::new(SyncQueue { path, gate }))
}

#[uniffi::export(async_runtime = "tokio")]
impl SyncQueue {
    pub async fn import_operations(
        &self,
        operations: Vec<QueuedOperation>,
    ) -> Result<(), CoreError> {
        let operations = operations
            .into_iter()
            .map(|operation| halogen_sync_journal::QueuedOperation {
                id: operation.id,
                operation_json: operation.operation_json,
            })
            .collect::<Vec<_>>();
        let operations = halogen_sync_journal::decode_operations(&operations).map_err(error)?;
        let path = self.path.clone();
        let guard = self.gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            NativeLocalStore::open(path)
                .and_then(|store| store.import_operations(&operations))
                .map_err(error)
        })
        .await
        .map_err(error)?
    }

    /// All entries are returned, including rejected work and its delivery metadata.
    pub async fn pending(&self) -> Result<String, CoreError> {
        let path = self.path.clone();
        let handle = tokio::runtime::Handle::current();
        let guard = self.gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let store = NativeLocalStore::open(path).map_err(error)?;
            let entries = handle.block_on(store.journal_entries()).map_err(error)?;
            serde_json::to_string(&entries).map_err(error)
        })
        .await
        .map_err(error)?
    }

    pub async fn pending_ids(&self) -> Result<Vec<String>, CoreError> {
        self.entry_ids(false).await
    }

    pub async fn quarantined_ids(&self) -> Result<Vec<String>, CoreError> {
        self.entry_ids(true).await
    }

    pub async fn delivered_ids(&self) -> Result<Vec<String>, CoreError> {
        let raw = self.pending().await?;
        let entries: Vec<(u64, halogen_sync_store::JournalEntry)> =
            serde_json::from_str(&raw).map_err(error)?;
        Ok(entries
            .into_iter()
            .filter(|(_, entry)| entry.delivered)
            .filter_map(|(_, entry)| entry.source_id)
            .collect())
    }

    /// Prune delivered payload only after the native cache has durably applied it.
    pub async fn confirm_cached(&self, source_ids: Vec<String>) -> Result<(), CoreError> {
        let path = self.path.clone();
        let handle = tokio::runtime::Handle::current();
        let guard = self.gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let store = NativeLocalStore::open(path).map_err(error)?;
            handle.block_on(async {
                let wanted = source_ids
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>();
                let ids = store
                    .journal_entries()
                    .await
                    .map_err(error)?
                    .into_iter()
                    .filter(|(_, entry)| {
                        entry.delivered
                            && entry
                                .source_id
                                .as_ref()
                                .is_some_and(|id| wanted.contains(id))
                    })
                    .map(|(id, _)| id)
                    .collect();
                store
                    .commit_changes(
                        &halogen_sync_store::StoreChanges {
                            acknowledged_operations: ids,
                            ..Default::default()
                        },
                        &[],
                    )
                    .await
                    .map_err(error)
            })
        })
        .await
        .map_err(error)?
    }

    pub async fn clear_all(&self) -> Result<(), CoreError> {
        let path = self.path.clone();
        let handle = tokio::runtime::Handle::current();
        let guard = self.gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let store = NativeLocalStore::open(path).map_err(error)?;
            handle.block_on(store.clear()).map_err(error)
        })
        .await
        .map_err(error)?
    }

    pub async fn drain_remote(
        &self,
        base_url: String,
        token: String,
    ) -> Result<SyncDrainReport, CoreError> {
        let api = remote_client(&base_url, token)?;
        self.drain(api).await
    }

    pub async fn drain_local(&self, core: Arc<LocalCore>) -> Result<SyncDrainReport, CoreError> {
        self.drain(ApiClient::local(core.session.clone())).await
    }
}

impl SyncQueue {
    async fn entry_ids(&self, rejected: bool) -> Result<Vec<String>, CoreError> {
        let raw = self.pending().await?;
        let entries: Vec<(u64, halogen_sync_store::JournalEntry)> =
            serde_json::from_str(&raw).map_err(error)?;
        Ok(entries
            .into_iter()
            .filter(|(_, entry)| entry.rejection.is_some() == rejected && !entry.delivered)
            .filter_map(|(_, entry)| entry.source_id)
            .collect())
    }

    async fn drain(&self, api: ApiClient) -> Result<SyncDrainReport, CoreError> {
        let path = self.path.clone();
        let handle = tokio::runtime::Handle::current();
        let guard = self.gate.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let store = NativeLocalStore::open(path).map_err(error)?;
            handle.block_on(async {
                let mut report = SyncDrainReport::default();
                for (id, entry) in store.journal_entries().await.map_err(error)? {
                    if entry.rejection.is_some() || entry.delivered {
                        continue;
                    }
                    let source_id = entry.source_id.clone();
                    match push_entry(&store, &api, id, entry).await.map_err(error)? {
                        PushOutcome::Applied(_) => {
                            report.applied += 1;
                            report.applied_ids.extend(source_id);
                        }
                        PushOutcome::Quarantined(error) => {
                            report.quarantined += 1;
                            report.quarantined_ids.extend(source_id);
                            report.last_error = Some(error.to_string());
                        }
                        PushOutcome::Retry { error, .. } => {
                            report.auth_paused =
                                halogen_sync_policy::status_of_error(&error) == Some(401);
                            report.last_error = Some(error.to_string());
                            break;
                        }
                    }
                }
                report.pending = store
                    .pending()
                    .await
                    .map_err(error)?
                    .len()
                    .try_into()
                    .map_err(error)?;
                Ok(report)
            })
        })
        .await
        .map_err(error)?
    }
}

fn remote_client(base_url: &str, token: String) -> Result<ApiClient, CoreError> {
    let url = url::Url::parse(base_url).map_err(error)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || token.is_empty()
    {
        return Err(error("invalid remote sync credentials"));
    }
    let api = ApiClient::new(url);
    api.set_token(Some(token));
    Ok(api)
}
