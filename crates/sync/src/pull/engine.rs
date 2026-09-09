use anyhow::Result;
use halogen_apiclient::ApiClient;
use halogen_sync_store::{LocalStore, StoreChanges};
use halogen_wire::SyncChangesParams;

pub enum PullOutcome {
    Deferred,
    Applied(Box<StoreChanges>),
}

/// Apply one bounded delta page; cache rows and the cursor share one transaction.
pub async fn pull_changes(store: &dyn LocalStore, api: &ApiClient) -> Result<PullOutcome> {
    if !store.pending().await?.is_empty() {
        return Ok(PullOutcome::Deferred);
    }
    let cursor = store.sync_cursor().await?;
    let delta = api
        .get_sync_changes(SyncChangesParams {
            cursor: cursor.clone(),
            limit: Some(500),
        })
        .await?;
    let mut changes = if delta.reset {
        super::snapshot::fetch_consistent_snapshot(api, delta.next_cursor.clone()).await?
    } else {
        super::resources::fetch_changes(store, api, &delta.changes).await?
    };
    if changes.sync_cursor.is_none() {
        changes.sync_cursor = Some(delta.next_cursor);
    }
    changes.require_empty_pending = true;
    changes.check_sync_cursor = true;
    changes.expected_sync_cursor = cursor;
    if !store.pending().await?.is_empty() {
        return Ok(PullOutcome::Deferred);
    }
    store.commit_changes(&changes, &[]).await?;
    Ok(PullOutcome::Applied(Box::new(changes)))
}
