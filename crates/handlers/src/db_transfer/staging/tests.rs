use super::{staging_dir, staging_file};

#[test]
fn private_files_refuse_overwrite() {
    let dir = staging_dir("test").unwrap();
    let path = dir.join("import.db");
    let file = staging_file(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(dir.metadata().unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(
        staging_file(&path).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    drop(file);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn staging_file_refuses_symlinks() {
    let dir = staging_dir("symlink-test").unwrap();
    let target = dir.join("target");
    std::fs::write(&target, b"keep").unwrap();
    let link = dir.join("import.db");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(staging_file(&link).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn sqlite_snapshot_accepts_private_empty_file() {
    let dir = staging_dir("snapshot-test").unwrap();
    let path = dir.join("export.db");
    drop(staging_file(&path).unwrap());
    let db = halogen_migrations::get_dbc(&dir.join("source.db"))
        .await
        .unwrap();
    halogen_queries::snapshot::create_snapshot(&db, &path)
        .await
        .unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    db.close().await.unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
