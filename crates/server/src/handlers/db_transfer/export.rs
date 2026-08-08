//! Build the downloadable DB export: a WAL-safe snapshot with secrets and
//! machine-local state scrubbed, gzipped.
//!
//! `VACUUM INTO` produces a consistent point-in-time copy regardless of the
//! live DB's journal mode (WAL or rollback) — no sidecar files, no torn pages.
//! The scrub then runs on the COPY only:
//! - `user.password_hash` cleared (import re-provisions random passwords);
//! - every episode reset to NOT_DOWNLOADED with all download/file fields
//!   cleared, and art cache paths cleared — media files don't travel with the
//!   DB, so nothing in an export may claim to exist on disk;
//! - operational history dropped (poll jobs + sync/download error logs).

use std::fs;

use anyhow::{Context, Result, anyhow};
use flate2::Compression;
use flate2::write::GzEncoder;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

/// The scrub statements, run against the snapshot copy. Table names use the
/// singular `user` (the workspace invariant).
const SCRUB_SQL: &[&str] = &[
    "UPDATE user SET password_hash = ''",
    "UPDATE episode SET download_status = 'NOT_DOWNLOADED', content_file_path = NULL, \
     download_size = NULL, downloaded_at = NULL, download_started_at = NULL, \
     download_attempts = 0, art_file_path = NULL",
    "UPDATE podcast SET art_file_path = NULL",
    "DELETE FROM poll_job_podcast",
    "DELETE FROM poll_job",
    "DELETE FROM podcast_sync_error",
    "DELETE FROM episode_download_error",
    // Compact the copy after the deletes (it's about to go over the wire).
    "VACUUM",
];

/// Snapshot + scrub + gzip. Returns the compressed bytes.
pub async fn build_export(dbc: &DatabaseConnection) -> Result<Vec<u8>> {
    let dir = super::staging_dir("export");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let result = build_in(dbc, &dir).await;
    // Best-effort cleanup either way; the bytes are already in memory.
    let _ = fs::remove_dir_all(&dir);
    result
}

async fn build_in(dbc: &DatabaseConnection, dir: &std::path::Path) -> Result<Vec<u8>> {
    let snapshot = dir.join("halogen-export.db");
    let snapshot_str = snapshot
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 temp path"))?
        // SQLite string-literal escaping (temp paths won't contain quotes, but
        // never build SQL on that assumption).
        .replace('\'', "''");

    let backend = dbc.get_database_backend();
    dbc.execute_raw(Statement::from_string(
        backend,
        format!("VACUUM INTO '{snapshot_str}'"),
    ))
    .await
    .context("VACUUM INTO snapshot")?;

    // Scrub the copy through its own short-lived connection.
    let copy = halogen_migrate::get_dbc(&snapshot)
        .await
        .context("opening the snapshot for scrubbing")?;
    let copy_backend = copy.get_database_backend();
    for sql in SCRUB_SQL {
        copy.execute_raw(Statement::from_string(copy_backend, (*sql).to_string()))
            .await
            .with_context(|| format!("scrubbing export: {sql}"))?;
    }
    let _ = copy.close().await;

    // Read + gzip are blocking (file I/O + CPU) — off the reactor: the
    // embedded server shares the app's runtime, so blocking a worker here
    // would jank the UI for the duration of a large export.
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
