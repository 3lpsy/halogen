use halogen_integ::*;
use halogen_sync::{PullOutcome, PushOutcome, cached_snapshot, pull_changes, push_entry};
use halogen_sync_store::{LocalStore, NativeLocalStore, OutboxOp};
use halogen_wire::{PlaybackStoreData, PlaylistStoreData, PlaylistUpdateData};

#[tokio::test]
async fn delta_pull_survives_restart_preserves_pending_intent_and_removes_deleted_rows() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    app.seed_playlist("Queue", true).await;
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    let playlist = client
        .create_playlist(PlaylistStoreData {
            name: "Listen later".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 120,
            completed: false,
        })
        .await
        .unwrap();

    let path = app.media_root().join("client-sync.sqlite");
    let store = NativeLocalStore::open(path.clone()).unwrap();
    assert!(cached_snapshot(&store).await.unwrap().is_none());
    assert!(matches!(
        pull_changes(&store, &client).await.unwrap(),
        PullOutcome::Applied(_)
    ));
    assert!(
        store
            .list_podcasts()
            .await
            .unwrap()
            .iter()
            .any(|p| p.id == podcast_id)
    );
    assert!(!store.list_episodes(podcast_id).await.unwrap().is_empty());
    assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 120);
    let initial_cursor = store.sync_cursor().await.unwrap().unwrap();
    drop(store);

    // The persisted cursor resumes after reopening the process cache.
    let store = NativeLocalStore::open(path).unwrap();
    assert_eq!(store.sync_cursor().await.unwrap().unwrap(), initial_cursor);
    client
        .update_playlist(
            playlist.id,
            PlaylistUpdateData {
                name: Some("Travel".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    client
        .upsert_playback(PlaybackStoreData {
            episode_id,
            cursor: 300,
            completed: false,
        })
        .await
        .unwrap();
    store
        .enqueue(&OutboxOp::SetCursor {
            episode_id,
            cursor: 20,
        })
        .await
        .unwrap();
    assert!(matches!(
        pull_changes(&store, &client).await.unwrap(),
        PullOutcome::Deferred
    ));
    assert_eq!(store.sync_cursor().await.unwrap().unwrap(), initial_cursor);
    assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 120);

    // A deliberate backward seek wins when the queued edit reaches the server.
    let (id, entry) = store.journal_entries().await.unwrap().remove(0);
    assert!(matches!(
        push_entry(&store, &client, id, entry).await.unwrap(),
        PushOutcome::Applied(_)
    ));
    assert!(matches!(
        pull_changes(&store, &client).await.unwrap(),
        PullOutcome::Applied(_)
    ));
    assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 20);
    assert_eq!(
        store
            .list_playlists()
            .await
            .unwrap()
            .iter()
            .find(|p| p.id == playlist.id)
            .unwrap()
            .name,
        "Travel"
    );
    let snapshot = cached_snapshot(&store).await.unwrap().unwrap();
    assert!(snapshot.reset_cache);
    assert_eq!(snapshot.playbacks[0].cursor, 20);

    client.delete_playlist(playlist.id).await.unwrap();
    client.delete_podcast(podcast_id).await.unwrap();
    assert!(matches!(
        pull_changes(&store, &client).await.unwrap(),
        PullOutcome::Applied(_)
    ));
    assert!(
        !store
            .list_playlists()
            .await
            .unwrap()
            .iter()
            .any(|p| p.id == playlist.id)
    );
    assert!(
        !store
            .list_podcasts()
            .await
            .unwrap()
            .iter()
            .any(|p| p.id == podcast_id)
    );
    assert!(store.list_episodes(podcast_id).await.unwrap().is_empty());
    assert!(store.list_playbacks().await.unwrap().is_empty());
}

#[tokio::test]
async fn non_owner_subscription_snapshots_without_private_auto_playlist_settings() {
    use sea_orm::{ActiveModelTrait, Set};
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let owner_client = api(&app, &admin.token);
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &owner_client, "sed_podcast.xml").await;
    let (listener_id, token) = app.seed_user("shared-listener").await;
    halogen_orm::user_podcast::ActiveModel {
        user_id: Set(listener_id),
        podcast_id: Set(podcast_id),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&app.dbc)
    .await
    .unwrap();
    let client = api(&app, &token);
    assert_eq!(
        status_of(
            &client
                .get_podcast_auto_playlists(podcast_id)
                .await
                .unwrap_err()
        ),
        403
    );

    let store = NativeLocalStore::open(app.media_root().join("listener-sync.sqlite")).unwrap();
    assert!(matches!(
        pull_changes(&store, &client).await.unwrap(),
        PullOutcome::Applied(_)
    ));
    assert!(store.sync_cursor().await.unwrap().is_some());
    assert!(
        store
            .list_podcasts()
            .await
            .unwrap()
            .iter()
            .any(|row| row.id == podcast_id)
    );
    assert!(
        store
            .list_episodes(podcast_id)
            .await
            .unwrap()
            .iter()
            .any(|row| row.id == episode_id)
    );
    assert!(store.list_auto_playlists().await.unwrap()[&podcast_id].is_empty());
}

#[tokio::test]
async fn subscribing_after_initial_sync_loads_existing_auto_playlist_settings() {
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);
    let podcast_id = app
        .seed_podcast("Existing feed", "https://example.com/existing.xml")
        .await;
    let playlist = client
        .create_playlist(PlaylistStoreData {
            name: "Auto queue".into(),
            is_default: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    app.seed_podcast_auto_playlist(podcast_id, playlist.id)
        .await;
    halogen_orm::user_podcast::Entity::delete_by_id((admin.id, podcast_id))
        .exec(&app.dbc)
        .await
        .unwrap();
    let store = NativeLocalStore::open(app.media_root().join("subscription-sync.sqlite")).unwrap();
    pull_changes(&store, &client).await.unwrap();
    assert!(store.list_podcasts().await.unwrap().is_empty());

    halogen_orm::user_podcast::ActiveModel {
        user_id: Set(admin.id),
        podcast_id: Set(podcast_id),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&app.dbc)
    .await
    .unwrap();
    pull_changes(&store, &client).await.unwrap();
    let links = store.list_auto_playlists().await.unwrap();
    assert_eq!(
        links[&podcast_id]
            .iter()
            .map(|row| row.playlist_id)
            .collect::<Vec<_>>(),
        vec![playlist.id]
    );
}
