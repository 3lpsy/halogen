//! Config journey — `GET /config` returns the reconciled runtime config to an
//! admin, with secrets structurally absent, and is forbidden to non-admins.
//! Driven through `ApiClient::get_config`.
//!
//! Run with: `cargo nextest run -p halogen-integ -E 'binary(config_flow)'`

use halogen_integ::*;

#[tokio::test]
async fn config_is_admin_only_and_sanitised() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Admin gets a populated, decoded `ConfigData`.
    let cfg = client.get_config().await.expect("admin reads config");
    assert!(!cfg.media_root.is_empty(), "media_root present");
    assert_eq!(
        cfg.public_url_path, "/",
        "reconciled config reflects the test defaults"
    );
    assert_eq!(
        cfg.subscription_no_sync_before, "2026-01-01",
        "the no-sync-before cutoff surfaces as YYYY-MM-DD (default)"
    );
    assert!(
        cfg.db_skip_default_playlist,
        "the test harness skips the startup Queue seed"
    );
    // `ConfigData` has no field for the JWT secret or admin password — secrets
    // are stripped at the type level, so there's nothing to leak here.

    // A non-admin (valid token, not admin) is forbidden.
    let (_id, token) = app.seed_user("plain").await;
    let err = api(&app, &token).get_config().await.unwrap_err();
    assert_eq!(status_of(&err), 403, "config is admin-only");

    // Anonymous is rejected by the auth layer first (401).
    let err = anon_api(&app).get_config().await.unwrap_err();
    assert_eq!(status_of(&err), 401, "config requires auth");
}
