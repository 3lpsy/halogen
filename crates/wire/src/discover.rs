//! Shared request/response types for the Discover endpoints
//! (`GET /discover/search`, `GET /discover/providers`).
//!
//! Discover is an online-only podcast search: the server fans a query out to
//! external directories ("providers", e.g. iTunes, gpodder.net), proxying every
//! outbound call so the client only ever talks to our own origin (CSP rule —
//! see [`crate`] consumers in the UI and `FRONTEND_CSP` in the server).
//!
//! Results are intentionally **bare**: title, feed URL, an optional ~3-line
//! description and author, plus the provider that returned them. There is no
//! artwork field by design — the server must never fetch or relay images here.
//!
//! These types live in `db` (not the server) so the API client and the server
//! share one definition.

use serde::{Deserialize, Serialize};

use super::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// A search provider — anything we can query for podcasts.
///
/// The wire form is lowercase (`"itunes"`, `"gpodder"`) and is also used as the
/// stable provider half of [`DiscoverResultItem::id`] on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoverProvider {
    Itunes,
    Gpodder,
}

impl DiscoverProvider {
    /// Lowercase wire id, matching the serde representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            DiscoverProvider::Itunes => "itunes",
            DiscoverProvider::Gpodder => "gpodder",
        }
    }

    /// Human label for the toggle chips and result/detail provider badge.
    pub fn label(&self) -> &'static str {
        match self {
            DiscoverProvider::Itunes => "iTunes",
            DiscoverProvider::Gpodder => "gpodder.net",
        }
    }
}

/// Query params for `GET /discover/search`.
///
/// `q` is the search term. `providers` filters to a subset; `None` or an empty
/// list means "every available provider".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoverSearchParams {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub providers: Option<Vec<DiscoverProvider>>,
}

#[typeshare]
/// One bare search result. **No artwork field by design.**
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoverResultItem {
    /// Synthetic, stable id computed server-side as `hex(sha1(provider \0 feed_url))`.
    /// Used as the detail-page key; the client never recomputes it.
    pub id: String,
    pub provider: DiscoverProvider,
    pub title: String,
    pub feed_url: String,
    /// Optional description (may be empty — e.g. iTunes returns none). Truncated
    /// server-side to a sane bound.
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
}

#[typeshare]
/// A single provider's failure, surfaced so the UI can toast (e.g. "gpodder failed")
/// without failing the whole search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoverProviderError {
    pub provider: DiscoverProvider,
    pub message: String,
}

#[typeshare]
/// Response body for `GET /discover/search`.
///
/// Partial-failure detail lives in [`Self::errors`] — search itself succeeds
/// (HTTP 200) even when every provider fails; `items` is then empty and `errors`
/// explains why. (The envelope's own `errors` field is reserved for request
/// validation.)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverSearchData {
    pub items: Vec<DiscoverResultItem>,
    #[serde(default)]
    pub errors: Vec<DiscoverProviderError>,
}

impl ResponsableData for DiscoverSearchData {}

/// One provider's availability for the toggle UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoverProviderInfo {
    pub id: DiscoverProvider,
    pub label: String,
    /// Whether the server can currently query this provider (e.g. a config-gated
    /// provider with missing credentials would be `false`).
    pub available: bool,
    /// Whether the toggle should start enabled when the user has no saved choice.
    pub default_enabled: bool,
}

/// Response body for `GET /discover/providers` — what the UI renders toggles for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DiscoverProvidersData {
    pub providers: Vec<DiscoverProviderInfo>,
}

impl ResponsableData for DiscoverProvidersData {}
