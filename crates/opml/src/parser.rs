//! Native OPML file reading. The pure parse/extract logic lives in
//! `halogen_utils::opml` (wasm-safe, shared with the frontend); this module only
//! adds the filesystem entrypoint.
use std::path::Path;

use halogen_utils::opml::{OpmlDocument, parse_opml_str};

pub fn parse_opml_file<P: AsRef<Path>>(path: P) -> Result<OpmlDocument, String> {
    let path = path.as_ref();
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read OPML file: {}", e))?;

    parse_opml_str(&content)
}
