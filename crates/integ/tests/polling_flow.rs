//! Polling journey — the background poller control plane via the `ApiClient`
//! methods (`poll_status`/`start_polling`/`stop_polling`/`poll_now`). The control
//! routes are **admin-only** and `status` is authed, so the journey runs as the
//! seeded admin: status (idle) → start → start-again is a conflict → stop →
//! status (idle) → manual poll → non-admin is forbidden. The harness uses a ZERO
//! poll interval, so a started service never actually ticks — we assert the
//! control-plane responses and the idle-flag transitions, not sync side effects.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(polling_flow)'`

use halogen_integ::*;

#[tokio::test]
async fn polling_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // 1) Idle on a fresh server — no task running.
    assert!(
        !client.poll_status().await.expect("status").running,
        "starts idle"
    );

    // 2) Start the service — acknowledged with the documented message.
    let started = client.start_polling().await.expect("start");
    assert_eq!(started.message, "Polling service started");

    // 3) Status is still reachable while a task exists. (With a ZERO interval the
    //    spawned loop exits immediately, so the `running` flag is racy — we assert
    //    the endpoint decodes, not a specific value.)
    let _ = client.poll_status().await.expect("status").running;

    // 4) Starting again while a task is registered is a conflict (409).
    let err = client.start_polling().await.unwrap_err();
    assert_eq!(status_of(&err), 409, "double-start => conflict");

    // 5) Stop — acknowledged, clearing the task.
    let stopped = client.stop_polling().await.expect("stop");
    assert_eq!(stopped.message, "Polling service stopped");

    // 6) Idle again once the task is cleared.
    assert!(
        !client.poll_status().await.expect("status").running,
        "idle after stop"
    );

    // 7) A manual poll runs a sync cycle synchronously and reports success (no
    //    podcasts subscribed, so it's a no-op sync).
    let polled = client.poll_now().await.expect("poll");
    assert_eq!(polled.message, "Poll completed successfully");

    // 8) The control routes are admin-only: a non-admin user is forbidden (403).
    let (_id, token) = app.seed_user("plain").await;
    let user = api(&app, &token);
    let err = user.start_polling().await.unwrap_err();
    assert_eq!(status_of(&err), 403, "non-admin start => forbidden");
}
