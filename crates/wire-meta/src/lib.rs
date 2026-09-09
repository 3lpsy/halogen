//! Meta types that support the wire DTOs: request/response envelopes, pagination
//! and ordering, includes for eager-loading, and the DB validation-error mapping.
//!
//! WASM-safe by default; the optional `db` feature adds the SeaORM-facing impls.

use halogen_utils::constants::VALIDATION_REQUEST_FIELD;

#[cfg(feature = "db")]
pub mod db;
pub mod get;
pub mod includes;
pub mod list;
pub mod order;
pub mod pagination;
pub mod request;
pub mod response;

#[cfg(test)]
mod tests;

// validators requires &'static str
// TODO figure this out
static FIELD_NAMES: &[&str] = &["name", "is_default"];
pub fn field_ref(name: &str) -> &'static str {
    FIELD_NAMES
        .iter()
        .find(|&&x| x == name)
        .unwrap_or(&VALIDATION_REQUEST_FIELD)
}

#[cfg(feature = "http")]
pub mod error;

pub mod api;
