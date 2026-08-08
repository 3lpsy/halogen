//! Admin-only OPML import/export endpoints.
//!
//! - `POST /api/v1/opml/import` — accept OPML XML, bare-insert the podcasts, then
//!   kick off a background feed sync so episodes populate.
//! - `GET  /api/v1/opml/export` — serialize the current subscriptions as OPML XML.
//!
//! Both are gated by the [`AdminUser`](crate::routers::extractors::AdminUser)
//! extractor (401 without a token, 403 for non-admins).

pub mod export;
pub mod import;
