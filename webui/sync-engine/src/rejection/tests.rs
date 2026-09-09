use crate::SyncService;
use halogen_sync_store::{LocalStore, NativeLocalStore, OutboxOp};
use halogen_wire::PlaylistData;
use std::rc::Rc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn rejection_repairs_membership_only_after_later_pending_work_finishes() {
    let path = std::env::temp_dir().join(format!("halogen-repair-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Rc::new(NativeLocalStore::open(path).unwrap());
    let (tx, _) = futures::channel::mpsc::unbounded();
    let mut service = SyncService::new(store.clone(), None, tx);
    let now = chrono::Utc::now();
    let mut playlist = PlaylistData {
        id: 1,
        name: "Queue".into(),
        description: None,
        is_default: true,
        position: 0,
        on_remove_delete_file_client: false,
        on_remove_delete_file_server: false,
        created_at: now,
        updated_at: now,
        episode_ids: Some(vec![7, 42]),
        episode_playlist: None,
    };
    service.cache_playlists(vec![playlist.clone()]).await;
    store
        .enqueue(&OutboxOp::AddToPlaylist {
            playlist_id: 1,
            episode_ids: vec![42],
            position: None,
        })
        .await
        .unwrap();
    let (rejected_id, mut rejected) = store.journal_entries().await.unwrap().remove(0);
    rejected.rejection = Some("access changed".into());
    store
        .save_journal_entry(rejected_id, &rejected)
        .await
        .unwrap();
    store
        .enqueue(&OutboxOp::SetCursor {
            episode_id: 99,
            cursor: 23,
        })
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    service.api_client = Some(halogen_apiclient::ApiClient::new(base));
    service.repair_rejections().await;
    assert!(service.repaired_rejections.is_empty());
    assert_eq!(service.playlists.episodes_by_playlist[&1], vec![7, 42]);
    for (id, _) in store.pending().await.unwrap() {
        store.ack(id).await.unwrap();
    }

    playlist.episode_ids = Some(vec![7]);
    let response = serde_json::json!({"data": playlist, "paginator": null}).to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = socket.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&chunk[..count]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(
            request.contains("includes"),
            "repair must request authoritative membership"
        );
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
    });
    service.repair_rejections().await;
    server.await.unwrap();
    assert_eq!(service.playlists.episodes_by_playlist[&1], vec![7]);
    assert_eq!(
        store.list_playlists().await.unwrap()[0].episode_ids,
        Some(vec![7])
    );
    assert!(service.repaired_rejections.contains(&rejected_id));
    assert_eq!(
        store.journal_entries().await.unwrap().len(),
        1,
        "rejected intent remains reviewable"
    );
}
