use halogen_fixture::test_support::TestRoot;
use halogen_local_runtime::{ApiRequest, Library};

#[tokio::test]
async fn local_dispatch_preserves_profile_and_requires_no_network_auth() {
    let mut root = TestRoot::new("local_dispatch");
    let library = Library::open(root.path()).await.unwrap();
    let session = library.session(None).await.unwrap();
    let id = session.user_id();
    let username = session.username().to_owned();
    let request = |path: &str| ApiRequest {
        method: "GET".into(),
        path: path.into(),
        query: None,
        body: None,
    };
    let response = session.invoke(request("/api/v1/podcasts")).await.unwrap();
    assert_eq!(response.status, 200);
    assert!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["data"].is_array()
    );
    assert_eq!(
        session
            .invoke(request("/api/v1/auth/login"))
            .await
            .unwrap()
            .status,
        404
    );
    assert!(
        session
            .invoke(request("https://example.com/api/v1/podcasts"))
            .await
            .is_err()
    );
    assert!(Library::open(root.path()).await.is_err());
    library.shutdown().await;
    drop(session);
    drop(library);
    let reopened = Library::open(root.path()).await.unwrap();
    let profile = reopened.session(Some(&username)).await.unwrap();
    assert_eq!(profile.user_id(), id);
    assert!(reopened.session(Some("missing-profile")).await.is_err());
    reopened.shutdown().await;
    root.mark_success();
}

#[tokio::test]
async fn existing_sessions_observe_profile_demotion_deletion_and_close() {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
    let mut root = TestRoot::new("local_authorization");
    let library = Library::open(root.path()).await.unwrap();
    let session = library.session(None).await.unwrap();
    let id = session.user_id();
    let db = halogen_migrations::connect_and_migrate_wal(&root.path().join("halogen.db"), false)
        .await
        .unwrap();
    let request = |path: &str| ApiRequest {
        method: "GET".into(),
        path: path.into(),
        query: None,
        body: None,
    };
    assert_eq!(
        session
            .invoke(request("/api/v1/admin/config"))
            .await
            .unwrap()
            .status,
        200
    );
    halogen_orm::user::ActiveModel {
        id: Set(id),
        is_admin: Set(false),
        ..Default::default()
    }
    .update(&db)
    .await
    .unwrap();
    assert_eq!(
        session
            .invoke(request("/api/v1/admin/config"))
            .await
            .unwrap()
            .status,
        403
    );
    halogen_orm::user::Entity::delete_by_id(id)
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(
        session
            .invoke(request("/api/v1/podcasts"))
            .await
            .unwrap()
            .status,
        401
    );
    assert!(session.audio_path(1).await.is_err());
    db.close().await.unwrap();
    library.close().await.unwrap();
    assert!(session.invoke(request("/api/v1/podcasts")).await.is_err());
    let reopened = Library::open(root.path()).await.unwrap();
    reopened.close().await.unwrap();
    root.mark_success();
}
