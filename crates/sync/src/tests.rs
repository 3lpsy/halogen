use crate::{PushOutcome, push_entry};
use halogen_apiclient::{ApiClient, DispatchFuture, LocalTransport};
use halogen_fixture::test_support::TestRoot;
use halogen_sync_store::{LocalStore, NativeLocalStore, OutboxOp};
use halogen_wire_meta::api::{ApiRequest, ApiResponse};
use std::sync::Arc;

struct Reject(u16);
impl LocalTransport for Reject {
    fn invoke(&self, _: ApiRequest) -> DispatchFuture<'_> {
        Box::pin(async move {
            Ok(ApiResponse {
                status: self.0,
                body: br#"{"error":"rejected"}"#.to_vec(),
            })
        })
    }
}

#[tokio::test]
async fn only_absence_requests_ack_a_missing_target() {
    let mut root = TestRoot::new("missing_target_replay");
    let store = NativeLocalStore::open(root.path().join("cache.db")).unwrap();
    let api = ApiClient::local(Arc::new(Reject(404)));
    store
        .enqueue(&OutboxOp::Unsubscribe { podcast_id: 1 })
        .await
        .unwrap();
    let (id, entry) = store.journal_entries().await.unwrap().remove(0);
    assert!(matches!(
        push_entry(&store, &api, id, entry).await.unwrap(),
        PushOutcome::Applied(_)
    ));
    store
        .enqueue(&OutboxOp::SetCursor {
            episode_id: 1,
            cursor: 30,
        })
        .await
        .unwrap();
    let (id, entry) = store.journal_entries().await.unwrap().remove(0);
    assert!(matches!(
        push_entry(&store, &api, id, entry).await.unwrap(),
        PushOutcome::Quarantined(_)
    ));
    assert!(store.pending().await.unwrap().is_empty());
    assert_eq!(store.journal_entries().await.unwrap().len(), 1);
    root.mark_success();
}

#[tokio::test]
async fn auth_failure_keeps_the_operation_and_retry_budget() {
    let mut root = TestRoot::new("auth_paused_queue");
    let store = NativeLocalStore::open(root.path().join("cache.db")).unwrap();
    let api = ApiClient::local(Arc::new(Reject(401)));
    store
        .enqueue(&OutboxOp::SetCursor {
            episode_id: 1,
            cursor: 30,
        })
        .await
        .unwrap();
    let (id, entry) = store.journal_entries().await.unwrap().remove(0);
    assert!(matches!(
        push_entry(&store, &api, id, entry).await.unwrap(),
        PushOutcome::Retry { attempts: 0, .. }
    ));
    assert_eq!(store.pending().await.unwrap().len(), 1);
    root.mark_success();
}

#[tokio::test]
async fn native_delivery_retains_payload_until_the_cache_checkpoint() {
    let mut root = TestRoot::new("native_cache_checkpoint");
    let store = NativeLocalStore::open(root.path().join("cache.db")).unwrap();
    let api = ApiClient::local(Arc::new(Reject(404)));
    store
        .import_operations(&[(
            "native-action".into(),
            OutboxOp::Unsubscribe { podcast_id: 1 },
        )])
        .unwrap();
    let (id, entry) = store.journal_entries().await.unwrap().remove(0);
    assert!(matches!(
        push_entry(&store, &api, id, entry).await.unwrap(),
        PushOutcome::Applied(_)
    ));
    assert!(store.pending().await.unwrap().is_empty());
    assert!(store.journal_entries().await.unwrap()[0].1.delivered);
    store
        .commit_changes(
            &halogen_sync_store::StoreChanges {
                acknowledged_operations: vec![id],
                ..Default::default()
            },
            &[],
        )
        .await
        .unwrap();
    assert!(store.journal_entries().await.unwrap().is_empty());
    store
        .import_operations(&[(
            "native-action".into(),
            OutboxOp::Unsubscribe { podcast_id: 1 },
        )])
        .unwrap();
    assert!(store.pending().await.unwrap().is_empty());
    root.mark_success();
}
