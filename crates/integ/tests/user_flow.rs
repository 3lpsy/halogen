//! Exercise admin user list/get/update/delete through ApiClient. Seed a second user directly because there is no create
//! route and self-deletion returns 400. Run the halogen-integ user_flow binary.

use halogen_integ::*;
use halogen_wire::UserUpdateData;

#[tokio::test]
async fn user_journey() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // 1) Listing users shows the seeded admin.
    let page = client.list_users(list_all()).await.expect("list");
    assert_eq!(page.data.len(), 1, "only the admin exists initially");

    // 2) Fetch self by id.
    let me = client.get_user(admin.id).await.expect("get self");
    assert_eq!(me.id, admin.id, "self round-trips by id");
    assert!(me.is_admin, "admin flag set");

    // 3) Seed a second (non-admin) user; the list grows.
    let (other_id, _token) = app.seed_user("listener").await;
    let page = client.list_users(list_all()).await.expect("list");
    assert_eq!(page.data.len(), 2, "second user is listed");

    // 4) Update that user's username; the change persists.
    let updated = client
        .update_user(
            other_id,
            UserUpdateData {
                username: Some("listener_renamed".into()),
                is_admin: None,
            },
        )
        .await
        .expect("update");
    assert_eq!(updated.username, "listener_renamed");
    assert_eq!(
        client.get_user(other_id).await.expect("re-get").username,
        "listener_renamed",
        "update persisted"
    );

    // 5) Deleting *self* is refused (guard in the delete handler → 400).
    let err = client.delete_user(admin.id).await.unwrap_err();
    assert_eq!(status_of(&err), 400, "cannot delete the authed admin");

    // 6) Deleting the other user succeeds, leaving just the admin.
    client.delete_user(other_id).await.expect("delete other");
    let page = client.list_users(list_all()).await.expect("list");
    assert_eq!(page.data.len(), 1, "other user removed");
    assert_eq!(page.data[0].id, admin.id);
}

/// A non-admin self-updating with `is_admin=true` is rejected (403) by the
/// self-escalation guard in the user-update router; the flag never persists.
#[tokio::test]
async fn non_admin_cannot_self_escalate_is_admin() {
    let app = spawn().await;
    let _admin = app.seed_admin().await;
    let (user_id, user_token) = app.seed_user("climber").await;
    let user_client = api(&app, &user_token);

    // Sanity: starts non-admin.
    assert!(
        !user_client
            .get_user(user_id)
            .await
            .expect("get self")
            .is_admin,
        "seeded user starts non-admin"
    );

    // Granting itself admin is forbidden.
    let err = user_client
        .update_user(
            user_id,
            UserUpdateData {
                username: None,
                is_admin: Some(true),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 403, "non-admin may not grant itself admin");

    // The flag did NOT persist.
    assert!(
        !user_client
            .get_user(user_id)
            .await
            .expect("re-get")
            .is_admin,
        "is_admin stays false after the rejected self-escalation"
    );
}

/// The sole admin demoting themselves is rejected (400) by the last-admin
/// guard in the update handler — otherwise the admin surface is permanently
/// locked out. Once a second admin exists, the demotion goes through.
#[tokio::test]
async fn last_admin_cannot_self_demote() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // Sole admin flipping their own is_admin off → 400, flag untouched.
    let err = client
        .update_user(
            admin.id,
            UserUpdateData {
                username: None,
                is_admin: Some(false),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 400, "sole admin may not self-demote");
    assert!(
        client.get_user(admin.id).await.expect("re-get").is_admin,
        "admin flag survives the rejected demotion"
    );

    // Promote a second user to admin; the demotion now succeeds.
    let (other_id, _token) = app.seed_user("second_admin").await;
    client
        .update_user(
            other_id,
            UserUpdateData {
                username: None,
                is_admin: Some(true),
            },
        )
        .await
        .expect("promote a second admin");
    let updated = client
        .update_user(
            admin.id,
            UserUpdateData {
                username: None,
                is_admin: Some(false),
            },
        )
        .await
        .expect("demote with a second admin present");
    assert!(!updated.is_admin, "demotion applied");
}

/// 404 on update/get of a missing user id; 400 on a validation-invalid update
/// body (username shorter than the 3-char minimum) — round-tripped over the wire.
#[tokio::test]
async fn user_update_and_get_error_cases() {
    let app = spawn().await;
    let admin = app.seed_admin().await;
    let client = api(&app, &admin.token);

    // GET a never-existing user → 404.
    let err = client.get_user(999_999).await.unwrap_err();
    assert_eq!(status_of(&err), 404, "missing user get is 404");

    // UPDATE a never-existing user → 404.
    let err = client
        .update_user(
            999_999,
            UserUpdateData {
                username: Some("ghost".into()),
                is_admin: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(status_of(&err), 404, "missing user update is 404");

    // Validation 400: username below the 3-character minimum.
    let err = client
        .update_user(
            admin.id,
            UserUpdateData {
                username: Some("ab".into()),
                is_admin: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        status_of(&err),
        400,
        "too-short username is a validation 400"
    );
}
