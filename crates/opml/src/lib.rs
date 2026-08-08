//! OPML import. The pure parse/extract logic and data types live in the shared
//! (wasm-safe) `halogen_utils::opml`; this module only adds the DB-insert side
//! that turns parsed outlines into podcast rows (episodes come from a later sync)
//! plus a small filesystem entrypoint in `parser`.

pub mod import;
pub mod parser;

pub use import::{import_podcasts_from_opml, import_podcasts_from_opml_str, import_single_podcast};

#[cfg(test)]
mod tests;
