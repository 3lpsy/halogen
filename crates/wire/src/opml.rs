//! Shared request/response types for the admin OPML import/export endpoints (`POST /opml/import`, `GET
//! /opml/export`). These live here (not in the server) so the API client and the server share a single
//! definition. The OPML payload is the raw XML carried as a JSON string field; the actual parsing/serialization
//! lives in `halogen_utils::opml`.

use serde::{Deserialize, Serialize};
use validator::Validate;

use super::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// Request body for `POST /opml/import` — the raw OPML XML to import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
pub struct OpmlImportData {
    #[validate(length(min = 1, message = "OPML content is required"))]
    pub opml: String,
}

impl ResponsableData for OpmlImportData {}

#[typeshare]
/// Response body for `POST /opml/import` — a summary of what the import did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpmlImportResultData {
    #[typeshare(serialized_as = "U53")]
    pub created: usize,
    #[typeshare(serialized_as = "U53")]
    pub skipped: usize,
    #[typeshare(serialized_as = "U53")]
    pub errors: usize,
}

impl ResponsableData for OpmlImportResultData {}

#[typeshare]
/// Response body for `GET /opml/export` — the current subscriptions as OPML XML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpmlExportData {
    pub opml: String,
}

impl ResponsableData for OpmlExportData {}
