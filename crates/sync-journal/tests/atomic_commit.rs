#![cfg(not(target_arch = "wasm32"))]

use std::rc::Rc;

use futures::executor::block_on;
use halogen_sync_journal::BufferedStore;
use halogen_sync_store::{LocalStore, NativeLocalStore, OutboxOp};
use halogen_wire::PlaybackData;

fn playback(cursor: u64) -> PlaybackData {
    serde_json::from_value(serde_json::json!({
        "id": 1, "user_id": 1, "episode_id": 7, "cursor": cursor, "completed": false,
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

fn database(name: &str) -> (std::path::PathBuf, Rc<NativeLocalStore>) {
    let path =
        std::env::temp_dir().join(format!("halogen-journal-{}-{name}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Rc::new(NativeLocalStore::open(path.clone()).unwrap());
    (path, store)
}

#[test]
fn cache_and_coalesced_operation_wait_for_commit() {
    block_on(async {
        let (_path, store) = database("commit");
        store.save_playback(&playback(10)).await.unwrap();
        store
            .enqueue(&OutboxOp::SetCursor {
                episode_id: 7,
                cursor: 10,
            })
            .await
            .unwrap();
        let old_id = store.pending().await.unwrap()[0].0;
        let batch = BufferedStore::new(store.clone());
        batch.save_playback(&playback(20)).await.unwrap();
        batch.ack(old_id).await.unwrap();
        batch
            .enqueue(&OutboxOp::SetCursor {
                episode_id: 7,
                cursor: 20,
            })
            .await
            .unwrap();
        assert_eq!(batch.list_playbacks().await.unwrap()[0].cursor, 20);
        assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 10);
        assert_eq!(store.pending().await.unwrap()[0].0, old_id);
        batch.commit().await.unwrap();
        assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 20);
        let pending = store.pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(matches!(
            pending[0].1,
            OutboxOp::SetCursor { cursor: 20, .. }
        ));
    });
}

#[test]
fn rejected_journal_write_preserves_cache_and_previous_intent() {
    block_on(async {
        let (path, store) = database("rollback");
        store.save_playback(&playback(10)).await.unwrap();
        store
            .enqueue(&OutboxOp::SetCursor {
                episode_id: 7,
                cursor: 10,
            })
            .await
            .unwrap();
        let old_id = store.pending().await.unwrap()[0].0;
        // Fault injection exercises rollback after the cache and cursor coalescing writes.
        rusqlite::Connection::open(path).unwrap().execute_batch(
            "CREATE TRIGGER reject_journal BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'disk failure'); END;"
        ).unwrap();
        let batch = BufferedStore::new(store.clone());
        batch.save_playback(&playback(20)).await.unwrap();
        batch.ack(old_id).await.unwrap();
        batch
            .enqueue(&OutboxOp::SetCursor {
                episode_id: 7,
                cursor: 20,
            })
            .await
            .unwrap();
        assert!(batch.commit().await.is_err());
        assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 10);
        let pending = store.pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, old_id);
        assert!(matches!(
            pending[0].1,
            OutboxOp::SetCursor { cursor: 10, .. }
        ));
    });
}
