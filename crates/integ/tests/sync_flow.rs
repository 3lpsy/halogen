use halogen_integ::*;
use halogen_wire::{PlaylistStoreData, ResponseData, SyncChangesData, SyncResource};
use sea_orm::{ActiveModelTrait, ConnectionTrait, DbBackend, EntityTrait, Set, Statement};

async fn changes(app: &TestApp, token: &str, cursor: Option<&str>, limit: u64) -> SyncChangesData {
    let mut request = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/changes", app.base_url))
        .bearer_auth(token)
        .query(&[("limit", limit.to_string())]);
    if let Some(cursor) = cursor {
        request = request.query(&[("cursor", cursor)]);
    }
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 200);
    response
        .json::<ResponseData<SyncChangesData>>()
        .await
        .unwrap()
        .data
        .unwrap()
}

#[tokio::test]
async fn sync_cursor_is_scoped_paginated_and_recovers_after_retention() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let (_, token) = app.seed_user("delta-reader").await;
    let client = api(&app, &token);
    let initial = changes(&app, &token, None, 1).await;
    assert!(initial.reset && initial.changes.is_empty());
    let other = api(&app, &admin.token)
        .create_playlist(PlaylistStoreData {
            name: "Other user".into(),
            is_default: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let first = client
        .create_playlist(PlaylistStoreData {
            name: "Queue".into(),
            is_default: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let second = client
        .create_playlist(PlaylistStoreData {
            name: "Travel".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut cursor = initial.next_cursor.clone();
    let mut seen = vec![];
    for _ in 0..20 {
        let page = changes(&app, &token, Some(&cursor), 1).await;
        assert!(!page.reset);
        assert!(page.changes.len() <= 1);
        cursor = page.next_cursor;
        seen.extend(page.changes);
        if !page.has_more {
            break;
        }
    }
    assert!(seen.iter().any(|e| e.resource_id == first.id));
    assert!(seen.iter().any(|e| e.resource_id == second.id));
    assert!(
        seen.iter()
            .all(|e| e.resource == SyncResource::Playlists && e.resource_id != other.id)
    );
    assert!(
        seen.windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    client.delete_playlist(second.id).await.unwrap();
    let deleted = changes(&app, &token, Some(&cursor), 500).await;
    assert!(
        deleted
            .changes
            .iter()
            .any(|e| e.resource_id == second.id && e.deleted)
    );
    assert!(
        changes(&app, &token, Some(&deleted.next_cursor), 500)
            .await
            .changes
            .is_empty()
    );
    // Age the retained history, then verify a client cannot silently miss its tombstones.
    app.dbc
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE sync_change SET created_at = '2000-01-01 00:00:00'",
            [],
        ))
        .await
        .unwrap();
    assert!(
        changes(&app, &token, Some(&initial.next_cursor), 500)
            .await
            .reset
    );
    let wrong_epoch = format!("{}:0", "0".repeat(32));
    assert!(changes(&app, &token, Some(&wrong_epoch), 500).await.reset);
    let url = format!("{}/api/v1/sync/changes", app.base_url);
    let http = reqwest::Client::new();
    assert_eq!(http.get(&url).send().await.unwrap().status(), 401);
    for query in ["limit=501", "limit=0", "cursor=bad"] {
        assert_eq!(
            http.get(format!("{url}?{query}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
}

#[tokio::test]
async fn feed_and_user_state_changes_produce_scoped_invalidations() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let (other_id, other_token) = app.seed_user("other-reader").await;
    let initial = changes(&app, &admin.token, None, 500).await;
    let other_initial = changes(&app, &other_token, None, 500).await;
    let client = api(&app, &admin.token);
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    client
        .upsert_playback(halogen_wire::PlaybackStoreData {
            episode_id,
            cursor: 10,
            completed: false,
        })
        .await
        .unwrap();
    let page = changes(&app, &admin.token, Some(&initial.next_cursor), 500).await;
    for (resource, id) in [
        (SyncResource::Podcasts, podcast_id),
        (SyncResource::Episodes, episode_id),
        (SyncResource::Playbacks, episode_id),
    ] {
        assert!(
            page.changes
                .iter()
                .any(|e| e.resource == resource && e.resource_id == id && !e.deleted)
        );
    }
    assert!(
        changes(&app, &other_token, Some(&other_initial.next_cursor), 500)
            .await
            .changes
            .is_empty()
    );
    // A new subscription must deliver already-existing episodes, not only future feed updates.
    halogen_orm::user_podcast::ActiveModel {
        user_id: Set(other_id),
        podcast_id: Set(podcast_id),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
    }
    .insert(&app.dbc)
    .await
    .unwrap();
    let subscribed = changes(&app, &other_token, Some(&other_initial.next_cursor), 500).await;
    assert!(
        subscribed
            .changes
            .iter()
            .any(|e| e.resource == SyncResource::Episodes && e.resource_id == episode_id)
    );
    halogen_orm::user_podcast::Entity::delete_by_id((other_id, podcast_id))
        .exec(&app.dbc)
        .await
        .unwrap();
    let unsubscribed = changes(&app, &other_token, Some(&subscribed.next_cursor), 500).await;
    assert!(
        unsubscribed
            .changes
            .iter()
            .any(|e| e.resource == SyncResource::Podcasts
                && e.resource_id == podcast_id
                && e.deleted)
    );
    client.delete_podcast(podcast_id).await.unwrap();
    let deleted = changes(&app, &admin.token, Some(&page.next_cursor), 500).await;
    assert!(
        deleted
            .changes
            .iter()
            .any(|e| e.resource == SyncResource::Podcasts
                && e.resource_id == podcast_id
                && e.deleted)
    );
}

#[tokio::test]
async fn auto_playlist_changes_reach_subscribed_admins_only() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let (owner_id, owner_token) = app.seed_user("auto-owner").await;
    let (reader_id, reader_token) = app.seed_user("auto-reader").await;
    let podcast_id = app
        .seed_podcast("Auto links", "https://example.com/feed.xml")
        .await;
    halogen_orm::podcast::ActiveModel {
        id: Set(podcast_id),
        owner_id: Set(owner_id),
        ..Default::default()
    }
    .update(&app.dbc)
    .await
    .unwrap();
    for user_id in [owner_id, reader_id] {
        halogen_orm::user_podcast::ActiveModel {
            user_id: Set(user_id),
            podcast_id: Set(podcast_id),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        }
        .insert(&app.dbc)
        .await
        .unwrap();
    }
    let owner = api(&app, &owner_token);
    let playlist = owner
        .create_playlist(PlaylistStoreData {
            name: "Owner queue".into(),
            is_default: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut cursor = changes(&app, &admin.token, None, 500).await.next_cursor;
    let reader_cursor = changes(&app, &reader_token, None, 500).await.next_cursor;
    for playlist_ids in [vec![playlist.id], vec![]] {
        owner
            .set_podcast_auto_playlists(podcast_id, playlist_ids.clone(), None)
            .await
            .unwrap();
        let delta = changes(&app, &admin.token, Some(&cursor), 500).await;
        assert!(
            delta
                .changes
                .iter()
                .any(|event| event.resource == SyncResource::PodcastAutoPlaylists
                    && event.resource_id == podcast_id
                    && !event.deleted)
        );
        cursor = delta.next_cursor;
        let rows = api(&app, &admin.token)
            .get_podcast_auto_playlists(podcast_id)
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.playlist_id).collect::<Vec<_>>(),
            playlist_ids
        );
    }
    assert!(
        changes(&app, &reader_token, Some(&reader_cursor), 500)
            .await
            .changes
            .is_empty()
    );
    assert!(
        matches!(api(&app, &reader_token).get_podcast_auto_playlists(podcast_id).await,
        Err(error) if status_of(&error) == 403)
    );
}
