//! Shared OPML core — parsing, podcast extraction, and serialization.
//!
//! Everything here is **pure** (no I/O, only `roxmltree` + string building), so
//! it compiles to wasm and is shared by the server (import/export handlers) and
//! the frontend (parse + validate an OPML file before upload). Native-only
//! helpers that touch the filesystem (e.g. reading an OPML file from a path)
//! live in the server crate, not here.

use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug)]
pub struct ImportPodcastResult {
    pub total: usize,
    pub created: usize,
    pub skipped: usize,
    pub errors: usize,
    pub podcast_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpmlOutline {
    pub text: String,
    #[serde(rename = "type")]
    pub outline_type: Option<String>,
    pub xml_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpmlDocument {
    pub version: String,
    pub outlines: Vec<OpmlOutline>,
}

pub fn parse_opml_str(xml: &str) -> Result<OpmlDocument, String> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| format!("Failed to parse OPML document: {}", e))?;

    let root = doc.root_element();

    if root.tag_name().name() != "opml" {
        return Err("Root element is not 'opml'".to_string());
    }

    let version = root.attribute("version").unwrap_or("1.0").to_string();

    let mut outlines = Vec::new();
    for child in root.children() {
        if child.tag_name().name() == "body" {
            for outline in child.children() {
                if outline.tag_name().name() != "outline" {
                    continue;
                }
                let text = outline.attribute("text").unwrap_or("").to_string();
                let outline_type = outline.attribute("type").map(|s| s.to_string());
                let xml_url = outline.attribute("xmlUrl").map(|s| s.to_string());

                outlines.push(OpmlOutline {
                    text,
                    outline_type,
                    xml_url,
                });
            }
        }
    }

    debug!("Parsed OPML document ({} bytes)", xml.len());
    Ok(OpmlDocument { version, outlines })
}

pub fn extract_podcasts_from_opml(opml: &OpmlDocument) -> Vec<(String, String)> {
    let mut podcasts = Vec::new();

    for outline in &opml.outlines {
        if outline.outline_type.as_deref() == Some("rss")
            && outline
                .xml_url
                .as_deref()
                .is_some_and(|url| !url.is_empty())
        {
            let title = outline.text.clone();
            let feed_url = outline.xml_url.clone().unwrap();

            debug!("Found podcast: {} -> {}", title, feed_url);
            podcasts.push((title, feed_url));
        }
    }

    podcasts
}

/// Serialize an [`OpmlDocument`] to an OPML 2.0 XML string.
///
/// Attribute values are XML-escaped so titles/URLs containing `& < > "` round-trip
/// through [`parse_opml_str`]. Outlines without a `type` or `xmlUrl` omit those
/// attributes entirely.
pub fn to_opml_xml(doc: &OpmlDocument) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<opml version=\"{}\">\n",
        escape_attr(&doc.version)
    ));
    out.push_str("  <head/>\n");
    out.push_str("  <body>\n");
    for outline in &doc.outlines {
        out.push_str("    <outline text=\"");
        out.push_str(&escape_attr(&outline.text));
        out.push('"');
        if let Some(ty) = &outline.outline_type {
            out.push_str(" type=\"");
            out.push_str(&escape_attr(ty));
            out.push('"');
        }
        if let Some(url) = &outline.xml_url {
            out.push_str(" xmlUrl=\"");
            out.push_str(&escape_attr(url));
            out.push('"');
        }
        out.push_str("/>\n");
    }
    out.push_str("  </body>\n");
    out.push_str("</opml>\n");
    out
}

/// Escape a string for use inside a double-quoted XML attribute value.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"<?xml version="1.0"?>
<opml version="2.0">
  <head><title>Test</title></head>
  <body>
    <outline text="Podcast A" type="rss" xmlUrl="https://feeds.example.com/a"/>
    <outline text="Just Text" type="text"/>
    <outline text="Podcast B" type="rss" xmlUrl="https://feeds.example.com/b"/>
    <outline text="No URL" type="rss" xmlUrl=""/>
    <outline text="YouTube" type="youtube" xmlUrl="https://youtube.com/x"/>
  </body>
</opml>"#;

    // --- parse_opml_str ------------------------------------------------------

    #[test]
    fn parse_valid_document() {
        let doc = parse_opml_str(VALID).expect("should parse");
        assert_eq!(doc.version, "2.0");
        // Only <outline> children are collected (head/title ignored).
        assert_eq!(doc.outlines.len(), 5);
        assert_eq!(doc.outlines[0].text, "Podcast A");
        assert_eq!(doc.outlines[0].outline_type.as_deref(), Some("rss"));
        assert_eq!(
            doc.outlines[0].xml_url.as_deref(),
            Some("https://feeds.example.com/a")
        );
        // Missing attributes become defaults/None.
        assert_eq!(doc.outlines[1].text, "Just Text");
        assert_eq!(doc.outlines[1].xml_url, None);
    }

    #[test]
    fn parse_version_defaults_when_absent() {
        let xml = r#"<?xml version="1.0"?>
<opml><body></body></opml>"#;
        let doc = parse_opml_str(xml).expect("should parse");
        assert_eq!(doc.version, "1.0");
        assert!(doc.outlines.is_empty());
    }

    #[test]
    fn parse_non_opml_root_is_err() {
        let xml = r#"<?xml version="1.0"?><rss><body></body></rss>"#;
        let err = parse_opml_str(xml).unwrap_err();
        assert!(
            err.contains("Root element is not 'opml'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_malformed_xml_is_err() {
        let xml = r#"<opml><body><outline text="oops"></body>"#; // unclosed tag
        let err = parse_opml_str(xml).unwrap_err();
        assert!(
            err.contains("Failed to parse OPML document"),
            "unexpected error: {err}"
        );
    }

    // --- extract_podcasts_from_opml ------------------------------------------

    #[test]
    fn extract_filters_to_rss_with_nonempty_url() {
        let doc = parse_opml_str(VALID).expect("should parse");
        let podcasts = extract_podcasts_from_opml(&doc);
        // Only the two type="rss" outlines with a non-empty xmlUrl survive.
        assert_eq!(
            podcasts,
            vec![
                (
                    "Podcast A".to_string(),
                    "https://feeds.example.com/a".to_string()
                ),
                (
                    "Podcast B".to_string(),
                    "https://feeds.example.com/b".to_string()
                ),
            ]
        );
    }

    #[test]
    fn extract_skips_empty_xml_url() {
        let xml = r#"<?xml version="1.0"?>
<opml version="1.0"><body>
  <outline text="Good" type="rss" xmlUrl="https://feeds.example.com/good"/>
  <outline text="Empty" type="rss" xmlUrl=""/>
</body></opml>"#;
        let doc = parse_opml_str(xml).expect("should parse");
        let podcasts = extract_podcasts_from_opml(&doc);
        assert_eq!(podcasts.len(), 1);
        assert_eq!(podcasts[0].0, "Good");
    }

    // --- to_opml_xml ---------------------------------------------------------

    #[test]
    fn serialize_roundtrips_through_parser() {
        let doc = OpmlDocument {
            version: "2.0".to_string(),
            outlines: vec![
                OpmlOutline {
                    text: "Podcast A".to_string(),
                    outline_type: Some("rss".to_string()),
                    xml_url: Some("https://feeds.example.com/a".to_string()),
                },
                OpmlOutline {
                    text: "Podcast B".to_string(),
                    outline_type: Some("rss".to_string()),
                    xml_url: Some("https://feeds.example.com/b".to_string()),
                },
            ],
        };
        let xml = to_opml_xml(&doc);
        let reparsed = parse_opml_str(&xml).expect("serialized OPML should parse");
        assert_eq!(reparsed, doc);
    }

    #[test]
    fn serialize_escapes_special_chars() {
        let doc = OpmlDocument {
            version: "2.0".to_string(),
            outlines: vec![OpmlOutline {
                text: r#"Tom & Jerry's "<show>""#.to_string(),
                outline_type: Some("rss".to_string()),
                xml_url: Some("https://example.com/feed?a=1&b=2".to_string()),
            }],
        };
        let xml = to_opml_xml(&doc);
        // Raw special chars must have been escaped in the attribute value.
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&quot;"));
        assert!(xml.contains("&lt;"));
        // And they survive a round-trip back to the original values.
        let reparsed = parse_opml_str(&xml).expect("escaped OPML should parse");
        assert_eq!(reparsed, doc);
    }
}
