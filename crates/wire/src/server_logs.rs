//! Response type for the admin server-log tail endpoint
//! (`GET /admin/server-logs`).
//!
//! Lives here (not in the server) so the API client and the server share a
//! single definition — the client deserialises exactly what the server emits.

use serde::{Deserialize, Serialize};

use super::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// A tail of the server's logs. `lines` is oldest → newest, capped server-side;
/// `path` is the configured log-file location, or `None` when the server isn't
/// logging to a file — the lines then come from the server's in-memory ring
/// (current process only, not persisted across restarts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerLogsData {
    pub path: Option<String>,
    pub lines: Vec<String>,
}

impl ResponsableData for ServerLogsData {}
