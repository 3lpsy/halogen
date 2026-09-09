use crate::db_transfer::DB_IMPORT_MAX_DECOMPRESSED_BYTES;
use flate2::read::GzDecoder;
use halogen_utils::{
    constants::{VALIDATION_CONFLICT_CODE, VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD},
    verrors,
};
use halogen_wire::{DbImportSummaryData, ValidationErrors};
use sea_orm::DatabaseConnection;
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub(super) fn bad_request(msg: impl Into<String>) -> ValidationErrors {
    verrors(
        VALIDATION_REQUEST_FIELD,
        VALIDATION_CONFLICT_CODE,
        msg.into(),
    )
}

/// Accept a gzipped or raw SQLite export and merge it into `dbc`.
pub async fn handle(
    dbc: &DatabaseConnection,
    body: &[u8],
) -> Result<DbImportSummaryData, ValidationErrors> {
    // Decompression + temp-file staging are blocking (CPU + file I/O, up to
    // the 1 GiB decompressed cap) — off the reactor, like the export side.
    let body = body.to_vec();
    let (path, dir) = tokio::task::spawn_blocking(move || stage_upload(&body))
        .await
        .map_err(|e| {
            verrors(
                VALIDATION_REQUEST_FIELD,
                VALIDATION_PANIC_CODE,
                format!("Import staging task failed: {e}"),
            )
        })??;
    let result = super::source::merge_from(dbc, &path).await;
    let _ = tokio::task::spawn_blocking(move || fs::remove_dir_all(&dir)).await;
    result
}

/// Gunzip (size-capped) or take the payload raw, then land it in a fresh temp
/// dir so SQLite can open it. Returns `(db file, temp dir to clean up)`.
fn stage_upload(body: &[u8]) -> Result<(PathBuf, PathBuf), ValidationErrors> {
    // Gunzip when the payload carries the gzip magic; otherwise take it raw.
    let raw = if body.len() >= 2 && body[..2] == GZIP_MAGIC {
        let mut out = Vec::new();
        // `take(cap + 1)`: a decompression bomb stops at the cap instead of
        // exhausting memory; landing above the cap means the payload overran it.
        let cap = DB_IMPORT_MAX_DECOMPRESSED_BYTES as u64;
        let mut decoder = GzDecoder::new(body).take(cap + 1);
        decoder
            .read_to_end(&mut out)
            .map_err(|e| bad_request(format!("Invalid gzip payload: {e}")))?;
        if out.len() as u64 > cap {
            return Err(bad_request(format!(
                "Decompressed import exceeds the {} MiB limit",
                DB_IMPORT_MAX_DECOMPRESSED_BYTES / (1024 * 1024)
            )));
        }
        out
    } else {
        body.to_vec()
    };
    if raw.len() < SQLITE_MAGIC.len() || &raw[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
        return Err(bad_request(
            "Not a SQLite database (expected a Halogen DB export, .db or .db.gz)",
        ));
    }

    // Land the upload in a temp file so SQLite can open it.
    let dir = crate::db_transfer::staging_dir("import")
        .map_err(|e| bad_request(format!("Failed to stage the import: {e}")))?;
    let path = dir.join("import.db");
    if let Err(e) =
        crate::db_transfer::staging_file(&path).and_then(|mut file| file.write_all(&raw))
    {
        let _ = fs::remove_dir_all(&dir);
        return Err(bad_request(format!("Failed to stage the import: {e}")));
    }
    Ok((path, dir))
}
