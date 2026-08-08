//! Podcast-config journey — the lifecycle of a polling/download config through
//! the podcast it belongs to: create+link → get → update → remove → confirm
//! gone. Configs are only ever created/deleted via their podcast (there is no
//! standalone create/list/delete). Driven through the `ApiClient`.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(podcast_config_flow)'`

use halogen_integ::*;
use halogen_wire::{PodcastConfigStoreData, PodcastConfigUpdateData, PodcastStoreData};

#[tokio::test]
async fn podcast_config_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Configs belong to a podcast — create one to attach the config to.
    let podcast = client
        .create_podcast(PodcastStoreData {
            title: "Configured".into(),
            description: None,
            feed_url: "https://example.com/configured-feed.xml".into(),
            art_url: None,
            author: None,
            podcast_config_id: None,
        })
        .await
        .expect("create podcast");

    // Create + link the config; the returned row echoes the values.
    let created = client
        .create_podcast_config_for(
            podcast.id,
            PodcastConfigStoreData {
                poll_interval_seconds: Some(300),
                max_episodes: Some(50),
                max_concurrent_downloads: Some(4),
                auto_download_enabled: Some(true),
            },
        )
        .await
        .expect("create + link");
    assert_eq!(created.auto_download_enabled, Some(true));
    assert_eq!(created.poll_interval_seconds, Some(300));
    assert_eq!(created.max_episodes, Some(50));
    let config_id = created.id;

    // Fetchable by id (GET /podcast-configs/{id}).
    let got = client.get_podcast_config(config_id).await.expect("get");
    assert_eq!(got.id, config_id);
    assert_eq!(got.poll_interval_seconds, Some(300));

    // Update one field; it persists (re-get confirms), others untouched.
    let updated = client
        .update_podcast_config(
            config_id,
            PodcastConfigUpdateData {
                poll_interval_seconds: Some(600),
                max_episodes: None,
                max_concurrent_downloads: None,
                auto_download_enabled: None,
            },
        )
        .await
        .expect("update");
    assert_eq!(updated.poll_interval_seconds, Some(600));
    assert_eq!(
        client
            .get_podcast_config(config_id)
            .await
            .expect("re-get")
            .poll_interval_seconds,
        Some(600),
        "update persisted"
    );

    // Remove via the podcast (unlink + delete); get-by-id then 404s.
    client
        .remove_podcast_config_for(podcast.id)
        .await
        .expect("remove");
    let err = client.get_podcast_config(config_id).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "deleted config is gone");

    // A never-existing id is also a 404.
    let err = client.get_podcast_config(999_999).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "missing config is 404");
}

/// Validation 400 on create: `max_episodes` outside the allowed 1..=10000 range
/// is rejected through the real wire (the create goes via the podcast).
#[tokio::test]
async fn podcast_config_validation_400() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    let podcast = client
        .create_podcast(PodcastStoreData {
            title: "Bad config".into(),
            description: None,
            feed_url: "https://example.com/bad-config-feed.xml".into(),
            art_url: None,
            author: None,
            podcast_config_id: None,
        })
        .await
        .expect("create podcast");

    // max_episodes = 0 is below the minimum of 1 → 400.
    let err = client
        .create_podcast_config_for(
            podcast.id,
            PodcastConfigStoreData {
                poll_interval_seconds: Some(300),
                max_episodes: Some(0),
                max_concurrent_downloads: Some(4),
                auto_download_enabled: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        status_of(&err),
        400,
        "out-of-range max_episodes is a validation 400"
    );
}

/// The podcast-scoped nested endpoints: `POST /podcasts/{id}/config` creates AND
/// links a config (so `get_podcast` returns it), refuses a second one, and
/// `DELETE /podcasts/{id}/config` unlinks + deletes it.
#[tokio::test]
async fn podcast_config_nested_link_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // A podcast with no config yet.
    let podcast = client
        .create_podcast(PodcastStoreData {
            title: "Nested".into(),
            description: None,
            feed_url: "https://example.com/nested-feed.xml".into(),
            art_url: None,
            author: None,
            podcast_config_id: None,
        })
        .await
        .expect("create podcast");
    let podcast_id = podcast.id;
    assert_eq!(podcast.podcast_config_id, None);

    // Create + link in one atomic call.
    let cfg = client
        .create_podcast_config_for(
            podcast_id,
            PodcastConfigStoreData {
                poll_interval_seconds: Some(900),
                max_episodes: Some(25),
                max_concurrent_downloads: Some(2),
                auto_download_enabled: Some(true),
            },
        )
        .await
        .expect("create + link config");

    // get_podcast now carries the FK and the eager-loaded config body.
    let got = client.get_podcast(podcast_id).await.expect("get podcast");
    assert_eq!(got.podcast_config_id, Some(cfg.id));
    let loaded = got.podcast_config.expect("config included");
    assert_eq!(loaded.poll_interval_seconds, Some(900));
    assert_eq!(loaded.max_episodes, Some(25));

    // A second create for the same podcast is rejected.
    let err = client
        .create_podcast_config_for(
            podcast_id,
            PodcastConfigStoreData {
                poll_interval_seconds: Some(120),
                max_episodes: Some(10),
                max_concurrent_downloads: Some(1),
                auto_download_enabled: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        status_of(&err),
        409,
        "podcast already has a config is a conflict"
    );

    // Editing the linked config still works through the shared endpoint.
    client
        .update_podcast_config(
            cfg.id,
            PodcastConfigUpdateData {
                poll_interval_seconds: Some(1800),
                max_episodes: None,
                max_concurrent_downloads: None,
                auto_download_enabled: None,
            },
        )
        .await
        .expect("update linked config");

    // Remove: unlinks the podcast and deletes the config row.
    client
        .remove_podcast_config_for(podcast_id)
        .await
        .expect("remove config");
    let got = client.get_podcast(podcast_id).await.expect("get podcast");
    assert_eq!(got.podcast_config_id, None, "podcast unlinked");
    assert!(got.podcast_config.is_none(), "config gone");
    let err = client.get_podcast_config(cfg.id).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "deleted config is gone");

    // Removing again is an idempotent no-op success.
    client
        .remove_podcast_config_for(podcast_id)
        .await
        .expect("remove is idempotent");

    // Create for a missing podcast → 404.
    let err = client
        .create_podcast_config_for(999_999, PodcastConfigStoreData::default())
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "missing podcast is 404");
}
