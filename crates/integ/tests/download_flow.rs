//! Download journey — the on-demand server-side fetch trigger and its removal.
//! Ingesting a feed creates episodes as NOT_DOWNLOADED; `trigger_download`
//! acknowledges immediately (202) and fetches in the background;
//! `remove_server_download` resets a downloaded episode. Driven through the
//! `ApiClient`, asserting the client-visible `download_status`.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(download_flow)'`

use halogen_integ::*;
use halogen_wire::{DownloadStatus, PlaybackStatus};

#[tokio::test]
async fn download_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Arrange: ingest an episode (created as NOT_DOWNLOADED).
    let (podcast_id, episode_id) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    assert_eq!(
        client
            .get_episode(episode_id, &[])
            .await
            .expect("get")
            .download_status,
        DownloadStatus::NotDownloaded,
        "freshly ingested episode is NotDownloaded"
    );

    // 1) No auth → the JWT layer rejects before the handler runs (401).
    let err = anon_api(&app)
        .trigger_download(episode_id)
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 401, "anonymous download => 401");

    // 2) Authed trigger is accepted (202 → Ok). The real fetch runs in a spawned
    //    task; we only assert the synchronous acknowledgement here.
    client.trigger_download(episode_id).await.expect("trigger");

    // 3) Removal path on a separate, deterministically "downloaded" episode (no
    //    background race): force Downloaded, then remove resets to NotDownloaded.
    let other = app
        .seed_episode(
            podcast_id,
            "Downloaded One",
            "",
            chrono::Utc::now(),
            PlaybackStatus::Unplayed,
        )
        .await;
    app.set_download_status(&[other], DownloadStatus::Downloaded)
        .await;
    assert_eq!(
        client
            .get_episode(other, &[])
            .await
            .expect("get")
            .download_status,
        DownloadStatus::Downloaded,
    );
    client
        .remove_server_download(other)
        .await
        .expect("remove download");
    assert_eq!(
        client
            .get_episode(other, &[])
            .await
            .expect("get")
            .download_status,
        DownloadStatus::NotDownloaded,
        "remove resets the episode to NotDownloaded"
    );
}

/// Bulk variants: `POST`/`DELETE /episodes/download/bulk` act on many ids in one
/// call. The trigger acknowledges (202 → Ok); the remove resets every authorized,
/// downloaded episode. Driven through the typed `ApiClient` as the single route is.
#[tokio::test]
async fn bulk_download_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Two episodes under an (admin-owned) feed.
    let (podcast_id, ep1) = subscribe_and_poll(&app, &client, "sed_podcast.xml").await;
    let ep2 = app
        .seed_episode(
            podcast_id,
            "Second",
            "",
            chrono::Utc::now(),
            PlaybackStatus::Unplayed,
        )
        .await;

    // Bulk trigger is accepted; the real fetches run in spawned tasks.
    client
        .trigger_download_bulk(vec![ep1, ep2])
        .await
        .expect("bulk trigger");

    // Bulk remove on SEPARATE, deterministically-downloaded episodes — NOT ep1/ep2,
    // whose background fetch (the unreachable "Second" feed fails and re-marks itself
    // DownloadError) would race the remove. Force Downloaded, then remove resets them
    // all to NotDownloaded. (Mirrors the single `download_journey`'s no-race path.)
    let rem1 = app
        .seed_episode(
            podcast_id,
            "Removable One",
            "",
            chrono::Utc::now(),
            PlaybackStatus::Unplayed,
        )
        .await;
    let rem2 = app
        .seed_episode(
            podcast_id,
            "Removable Two",
            "",
            chrono::Utc::now(),
            PlaybackStatus::Unplayed,
        )
        .await;
    app.set_download_status(&[rem1, rem2], DownloadStatus::Downloaded)
        .await;
    client
        .remove_server_download_bulk(vec![rem1, rem2])
        .await
        .expect("bulk remove");
    for id in [rem1, rem2] {
        assert_eq!(
            client
                .get_episode(id, &[])
                .await
                .expect("get")
                .download_status,
            DownloadStatus::NotDownloaded,
            "bulk remove resets every episode to NotDownloaded"
        );
    }
}

/// Authorization: the single AND bulk download routes are gated on subscription
/// (owner/subscriber/admin). A non-subscribed user is refused the single route
/// (404 — existence is hidden), and the bulk route silently *filters out* the ids
/// it isn't allowed to touch while still doing the rest.
#[tokio::test]
async fn download_subscription_guard_single_and_bulk() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let admin_client = api(&app, &admin.token);

    // An admin-owned, downloaded episode the plain user is NOT subscribed to.
    let (_podcast_id, ep) = subscribe_and_poll(&app, &admin_client, "sed_podcast.xml").await;
    app.set_download_status(&[ep], DownloadStatus::Downloaded)
        .await;

    let (_uid, plain_token) = app.seed_user("plain").await;
    let plain = api(&app, &plain_token);

    // Single route: the non-subscriber is refused with 404 (not 403 — the guard
    // hides existence from non-subscribers), for both the trigger and the remove.
    assert_eq!(
        status_of(&plain.trigger_download(ep).await.unwrap_err()),
        404,
        "single trigger by a non-subscriber must be 404"
    );
    assert_eq!(
        status_of(&plain.remove_server_download(ep).await.unwrap_err()),
        404,
        "single remove by a non-subscriber must be 404"
    );

    // Bulk route: accepted (the request shape is valid), but the unauthorized id is
    // filtered out — the episode stays Downloaded.
    plain
        .remove_server_download_bulk(vec![ep])
        .await
        .expect("bulk remove is accepted even with only-unauthorized ids");
    assert_eq!(
        admin_client
            .get_episode(ep, &[])
            .await
            .expect("get")
            .download_status,
        DownloadStatus::Downloaded,
        "an unauthorized id is filtered out of the bulk loop"
    );

    // The owner/admin bulk remove actually resets it — proving the 'no-op above was
    // authorization, not a broken endpoint.
    admin_client
        .remove_server_download_bulk(vec![ep])
        .await
        .expect("admin bulk remove");
    assert_eq!(
        admin_client
            .get_episode(ep, &[])
            .await
            .expect("get")
            .download_status,
        DownloadStatus::NotDownloaded,
        "the authorized bulk remove resets the episode"
    );
}
