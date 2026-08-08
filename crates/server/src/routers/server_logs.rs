//! Admin server-log tail (`GET /admin/server-logs`).
//!
//! Returns the last N lines of the server's log file (`Config.log_file`). The
//! server appends to that file for its whole life, so the handler reads only a
//! bounded window from the end — never the whole file. With no log file
//! configured, it falls back to the always-on in-memory ring
//! (`crate::logging::recent_lines`) — stdout can't be read back, but the ring
//! holds the same formatted lines for the current process.

use axum::{Json, extract::Extension, extract::Query};
use serde::Deserialize;

use halogen_wire::{ResponseData, ServerLogsData};

use crate::routers::extractors::AdminUser;

/// Where the running server writes its log file (from `Config.log_file`);
/// `None` when file logging is disabled. Layered as an `Extension` at router
/// construction.
#[derive(Clone)]
pub struct ServerLogsConfig {
    pub log_file: Option<std::path::PathBuf>,
}

/// Lines returned when the query doesn't ask for a count.
const DEFAULT_LINES: usize = 500;
/// Hard cap on the requested line count.
const MAX_LINES: usize = 2_000;
/// How much of the file's tail is read per request. Bounds memory + I/O on a
/// long-lived append-only log; at a typical ~150 bytes/line this comfortably
/// covers [`MAX_LINES`].
const MAX_READ_BYTES: u64 = 512 * 1024;

#[derive(Debug, Deserialize)]
pub struct TailParams {
    /// How many trailing lines to return (default [`DEFAULT_LINES`], capped at
    /// [`MAX_LINES`]).
    pub lines: Option<usize>,
}

/// GET /admin/server-logs — tail of the server logs. **Admin only.**
///
/// File-backed when `log_file` is configured (`path: Some`, full process
/// history); otherwise served from the in-memory ring (`path: None`, current
/// process only). A configured-but-unreadable file (e.g. nothing written yet)
/// yields empty lines rather than an error.
#[axum::debug_handler]
pub async fn get(
    _admin: AdminUser,
    Extension(cfg): Extension<ServerLogsConfig>,
    Query(params): Query<TailParams>,
) -> Json<ResponseData<ServerLogsData>> {
    let want = params.lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
    let (path, lines) = match cfg.log_file {
        None => (None, crate::logging::recent_lines(want)),
        Some(p) => {
            let lines = tail_file(&p, want).await.unwrap_or_default();
            (Some(p.display().to_string()), lines)
        }
    };
    Json(ResponseData::from_data(ServerLogsData { path, lines }))
}

/// Read the last `want` lines of `path`, oldest → newest, reading at most
/// [`MAX_READ_BYTES`] from the end of the file.
async fn tail_file(path: &std::path::Path, want: usize) -> std::io::Result<Vec<String>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path).await?;
    let len = file.metadata().await?.len();
    let start = len.saturating_sub(MAX_READ_BYTES);
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // A mid-file seek lands mid-line; drop the partial first line (only when the
    // window didn't cover the whole file).
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    let skip = lines.len().saturating_sub(want);
    Ok(lines.split_off(skip))
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::json_body;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn get_logs(router: &axum::Router, token: Option<&str>) -> axum::response::Response {
        let mut b = Request::builder()
            .method("GET")
            .uri("/api/v1/admin/server-logs");
        if let Some(t) = token {
            b = b.header("Authorization", format!("Bearer {t}"));
        }
        router
            .clone()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn server_logs_require_authentication() {
        let (_root, dbc, _payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        assert_eq!(
            get_logs(&router, None).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn server_logs_forbidden_for_non_admin() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let user = generate_jwt_token(payload["user_id"].as_str().unwrap());
        assert_eq!(
            get_logs(&router, Some(&user)).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn server_logs_admin_gets_memory_ring_when_no_file_configured() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);
        let admin = generate_jwt_token(payload["admin_id"].as_str().unwrap());
        // No file configured → served from the in-memory ring (`path: null`).
        // The test binary never installs the tracing subscriber, so the ring is
        // empty here — the fallback wiring itself is covered by the ring unit
        // tests in `crate::logging`.
        let resp = get_logs(&router, Some(&admin)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert!(json["data"]["path"].is_null());
        assert_eq!(json["data"]["lines"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn tail_file_returns_last_lines_in_order() {
        let mut root = halogen_fixture::test_support::TestRoot::new("server_logs_tail");
        let path = root.path().join("halogen.log");
        let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, content).unwrap();

        let tail = super::tail_file(&path, 3).await.unwrap();
        assert_eq!(tail, vec!["line 8", "line 9", "line 10"]);

        // Asking for more lines than exist returns the whole file.
        let all = super::tail_file(&path, 100).await.unwrap();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0], "line 1");
        root.mark_success();
    }
}
