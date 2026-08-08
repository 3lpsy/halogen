//! Default list parameter types.
//!
//! These are generic parameter structs used by list endpoints across all entities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use validator::Validate;

use super::{
    includes::{HasIncludes, Includable},
    order::{HasOrder, Order},
    pagination::{HasPagination, Pagination},
};

/// Filter parameters for list endpoints.
///
/// Optional on every field so existing callers are unaffected.
#[derive(Debug, Clone, Default, Validate, Serialize, Deserialize)]
pub struct FilterParams {
    /// Title contains (ILIKE)
    pub search: Option<String>,
    /// Filter by podcast ID
    pub podcast_id: Option<i32>,
    /// Filter by download status
    pub download_status: Option<String>,
    /// Only episodes published after this timestamp
    pub published_after: Option<DateTime<Utc>>,
    /// Filter by per-user listen state (`UNPLAYED`/`PLAYED`/`FINISHED`).
    pub playback_status: Option<String>,
    /// Restrict to a specific set of episode ids (e.g. the client's
    /// device-download set). Bounded to keep the SQL `IN (…)` clause sane.
    #[validate(length(max = 4096, message = "Too many ids"))]
    pub ids: Option<Vec<i32>>,
    /// Deprecated — superseded by `playback_status`. Kept for back-compat; unused.
    pub unplayed_only: Option<bool>,
}

/// Default list parameters: pagination + order + includes + filters.
///
/// Generic over `T: Includable + Serialize + DeserializeOwned` so any entity can use
/// its own include type.
#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
#[serde(bound = "T: Serialize + for<'a> Deserialize<'a>")]
pub struct DefaultListParams<T: Includable + Serialize + DeserializeOwned> {
    #[serde(default)]
    #[validate(nested)]
    pub pagination: Option<Pagination>,
    #[serde(default)]
    #[validate(nested)]
    pub order: Option<Order>,
    #[serde(default)]
    #[validate(length(max = 10, message = "Max 10 includes allowed"))]
    pub includes: Option<Vec<T>>,
    #[serde(default)]
    #[validate(nested)]
    pub filter: Option<FilterParams>,
}

impl<T: Includable + Serialize + DeserializeOwned> HasPagination for DefaultListParams<T> {
    fn pagination(&mut self) -> &mut Option<Pagination> {
        &mut self.pagination
    }
}

impl<T: Includable + Serialize + DeserializeOwned> HasOrder for DefaultListParams<T> {
    fn order(&mut self) -> &mut Option<Order> {
        &mut self.order
    }
}

impl<T: Includable + Serialize + DeserializeOwned> HasIncludes<T> for DefaultListParams<T> {
    fn includes(&mut self) -> &mut Option<Vec<T>> {
        &mut self.includes
    }
}
