//! Export a gzipped, WAL-safe VACUUM INTO snapshot. Scrub only the copy: clear password hashes, download flags and
//! media/art paths, and operational history. Import provisions new passwords; media files do not travel with the DB.

use std::fs;

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;
use sea_orm::DatabaseConnection;

/// Snapshot + scrub + gzip. Returns the compressed bytes.
pub async fn build_export(dbc: &DatabaseConnection) -> Result<Vec<u8>> {
    let dir = super::staging_dir("export").context("creating private export directory")?;

    let result = build_in(dbc, &dir).await;
    // Best-effort cleanup either way; the bytes are already in memory.
    let _ = fs::remove_dir_all(&dir);
    result
}

async fn build_in(dbc: &DatabaseConnection, dir: &std::path::Path) -> Result<Vec<u8>> {
    let snapshot = dir.join("halogen-export.db");
    // VACUUM INTO accepts an empty file; create it privately before SQLite writes credentials.
    drop(super::staging_file(&snapshot).context("creating private export snapshot")?);
    halogen_queries::snapshot::create_snapshot(dbc, &snapshot)
        .await
        .context("creating export snapshot")?;

    // Scrub the copy through its own short-lived connection.
    let copy = halogen_migrations::get_dbc(&snapshot)
        .await
        .context("opening the snapshot for scrubbing")?;
    halogen_queries::snapshot::scrub_snapshot(&copy)
        .await
        .context("scrubbing export snapshot")?;
    let _ = copy.close().await;

    // Move file I/O and gzip off the async runtime so large local exports do not stall the app.
    let snapshot_owned = snapshot.clone();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let raw = fs::read(&snapshot_owned).context("reading the scrubbed snapshot")?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, &raw).context("compressing export")?;
        encoder.finish().context("finishing export compression")
    })
    .await
    .context("export compression task")??;
    Ok(bytes)
}
