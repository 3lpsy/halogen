//! Database / seeding unit tests (admin + default-playlist seeding behaviour).

use halogen_fixture::test_support::TestRoot;
use halogen_orm::user::Column;
use halogen_orm::user::Entity as UserEntity;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use halogen_fixture::playlist::seed_default_queue;
use halogen_fixture::user::seed_admin_user;
use halogen_migrations::connect_and_migrate;

#[tokio::test]
async fn test_seed_admin_creates_user_when_none_exist() {
    let mut root = TestRoot::new("admin_seed");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true).await.unwrap();

    let (created, pw) = seed_admin_user(&dbc, "dev_admin", Some("dev_password_123"))
        .await
        .unwrap();

    assert!(created, "admin should be created");
    assert_eq!(pw, "dev_password_123");

    let admin = UserEntity::find()
        .filter(Column::IsAdmin.eq(true))
        .one(&dbc)
        .await
        .unwrap()
        .expect("admin user should exist");

    assert_eq!(admin.username, "dev_admin");
    assert!(
        admin.password_hash.starts_with("$2"),
        "password must be bcrypt-hashed"
    );

    drop(dbc);
    root.mark_success();
}

#[tokio::test]
async fn test_seed_admin_skips_when_admin_already_exists() {
    let mut root = TestRoot::new("admin_seed");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true).await.unwrap();

    seed_admin_user(&dbc, "first_admin", Some("password1"))
        .await
        .unwrap();

    let result = seed_admin_user(&dbc, "second_admin", Some("password2"))
        .await
        .unwrap();

    assert!(!result.0, "second admin should not be created");

    let count = UserEntity::find()
        .filter(Column::IsAdmin.eq(true))
        .count(&dbc)
        .await
        .unwrap();

    assert_eq!(count, 1);

    drop(dbc);
    root.mark_success();
}

#[tokio::test]
async fn test_seed_admin_generates_random_password_when_not_provided() {
    let mut root = TestRoot::new("admin_seed_gen");
    let db_path = root.path().join("halogen.db");

    let dbc = connect_and_migrate(&db_path, true).await.unwrap();

    let (created, pw) = seed_admin_user(&dbc, "gen_admin", None).await.unwrap();

    assert!(created, "admin should be created");
    assert_eq!(
        pw.len(),
        halogen_utils::constants::WT_PASSWORD_LENGTH,
        "generated password must meet minimum length"
    );
    assert!(
        pw.chars()
            .all(|c| c.is_ascii_alphanumeric() || "!@#$%".contains(c)),
        "generates password must use valid charset"
    );

    let admin = UserEntity::find()
        .filter(Column::Username.eq("gen_admin"))
        .one(&dbc)
        .await
        .unwrap()
        .expect("password hash should exist");

    bcrypt::verify(&pw, &admin.password_hash).unwrap();

    drop(dbc);
    root.mark_success();
}

#[tokio::test]
async fn test_seed_default_queue_creates_then_is_idempotent() {
    use halogen_orm::playlist::{Column as PlaylistColumn, Entity as PlaylistEntity};

    let mut root = TestRoot::new("queue_seed");
    let db_path = root.path().join("halogen.db");
    let dbc = connect_and_migrate(&db_path, true).await.unwrap();

    // A default queue is owned by a user now, so seed an admin first.
    seed_admin_user(&dbc, "queue_admin", Some("password123"))
        .await
        .unwrap();

    // First run creates the "Queue" default playlist.
    assert!(seed_default_queue(&dbc).await.unwrap(), "queue created");
    let queue = PlaylistEntity::find()
        .filter(PlaylistColumn::IsDefault.eq(true))
        .one(&dbc)
        .await
        .unwrap()
        .expect("default playlist exists");
    assert_eq!(queue.name, "Queue");

    // Second run is a no-op — still exactly one default playlist.
    assert!(!seed_default_queue(&dbc).await.unwrap(), "idempotent");
    let count = PlaylistEntity::find()
        .filter(PlaylistColumn::IsDefault.eq(true))
        .count(&dbc)
        .await
        .unwrap();
    assert_eq!(count, 1, "no duplicate default playlist");

    drop(dbc);
    root.mark_success();
}
