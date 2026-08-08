//! Admin DB export/import — the server↔server / embedded↔server migration and
//! full-backup endpoints. Both admin-only; the heavy lifting lives in
//! `handlers::db_transfer`.

use axum::body::Bytes;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::Utc;
use halogen_wire::{DbImportSummaryData, ResponseData};
use sea_orm::DatabaseConnection;

use crate::handlers::db_transfer::{export as export_handler, import as import_handler};
use crate::routers::ApiError;
use crate::routers::extractors::AdminUser;
use halogen_utils::constants::{VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD};

/// Uploads are metadata-only SQLite files (no media travels in them), but a
/// long-lived library can still outgrow axum's 2 MB default body cap by a lot.
pub const DB_IMPORT_MAX_BYTES: usize = 256 * 1024 * 1024;

/// Decompressed-size cap for gzipped imports (4× the compressed cap): bounds a
/// gzip bomb while leaving headroom for SQLite's typical gzip ratio.
pub const DB_IMPORT_MAX_DECOMPRESSED_BYTES: usize = 4 * DB_IMPORT_MAX_BYTES;

/// GET /api/v1/admin/db/export — a gzipped, scrubbed snapshot of the database
/// (no password hashes, nothing marked downloaded, no operational history).
pub async fn export(
    Extension(dbc): Extension<DatabaseConnection>,
    _admin: AdminUser,
) -> Result<Response, ApiError> {
    let bytes = export_handler::build_export(&dbc).await.map_err(|e| {
        tracing::error!("DB export failed: {e:#}");
        ApiError::new(
            VALIDATION_REQUEST_FIELD,
            VALIDATION_PANIC_CODE,
            "Failed to build the database export".to_string(),
        )
    })?;

    // CompressionLayer interaction (verified benign, documenting for the next
    // reader): protected_routes wraps this in tower-http compression, which
    // would double-gzip for clients advertising Accept-Encoding. Neither real
    // client does the wrong thing: the native ApiClient builds reqwest WITHOUT
    // the gzip feature (no Accept-Encoding → no re-compression), and browsers
    // transparently strip the transfer-level Content-Encoding, leaving exactly
    // the .db.gz payload bytes.
    let filename = format!(
        "halogen-export-{}.db.gz",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// POST /api/v1/admin/db/import — merge an uploaded export (gzipped or raw
/// SQLite) into this database. See `handlers::db_transfer::import` for the
/// merge rules; the response summarizes what happened per entity.
pub async fn import(
    Extension(dbc): Extension<DatabaseConnection>,
    _admin: AdminUser,
    body: Bytes,
) -> Result<Json<ResponseData<DbImportSummaryData>>, ApiError> {
    let summary = import_handler::handle(&dbc, &body)
        .await
        .map_err(ApiError)?;
    Ok(Json(ResponseData::from_data(summary)))
}
