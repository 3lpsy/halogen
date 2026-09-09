//! Web UI logging facade, persistence and download integration.
mod download;
pub mod store;
pub use download::{download_bytes, download_logs, download_text};
pub use halogen_logging_client::*;
