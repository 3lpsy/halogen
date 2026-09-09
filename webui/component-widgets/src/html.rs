//! Parse untrusted feed HTML into typed Block/Inline values rendered through escaping rsx. Allow structural paragraphs,
//! headings, lists, bold/text emphasis, and only HTTP(S)/mailto links; drop other tags and attributes while retaining
//! text. Never use dangerous_inner_html.

/// An inline run inside a block.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Bold(String),
    /// A link with a vetted (`http`/`https`/`mailto`) href.
    Link {
        href: String,
        text: String,
    },
}

/// A top-level block.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A paragraph. `heading` marks `h1`–`h6` (the UI renders it bold).
    Paragraph { inlines: Vec<Inline>, heading: bool },
    /// A bullet list — each item is its own run of inlines.
    List(Vec<Vec<Inline>>),
}

/// Whether an `href` is safe to expose as a clickable link. Everything else
/// (`javascript:`, `data:`, relative/unknown schemes) is rendered as plain text.
pub fn is_safe_href(href: &str) -> bool {
    let h = href.trim().to_ascii_lowercase();
    h.starts_with("http://") || h.starts_with("https://") || h.starts_with("mailto:")
}

/// Parsing failed — the caller should fall back to rendering the raw string as
/// escaped text (tags visible) rather than a half-interpreted result.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError;

// ── Tokenizer ────────────────────────────────────────────────────────────────

enum Tok {
    Open { name: String, href: Option<String> },
    Close(String),
    Br,
    Text(String),
}

fn tokenize(input: &str) -> Result<Vec<Tok>, ParseError> {
    let mut toks = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut text_start = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if i > text_start {
                toks.push(Tok::Text(input[text_start..i].to_string()));
            }
            // An unterminated tag (`<` with no `>`) is malformed — bail so the
            // caller renders the raw input literally.
            let Some(rel) = input[i..].find('>') else {
                return Err(ParseError);
            };
            let end = i + rel;
            let inner = input[i + 1..end].trim();
            push_tag(&mut toks, inner);
            i = end + 1;
            text_start = i;
        } else {
            i += 1;
        }
    }
    if text_start < bytes.len() {
        toks.push(Tok::Text(input[text_start..].to_string()));
    }
    Ok(toks)
}

fn push_tag(toks: &mut Vec<Tok>, inner: &str) {
    if inner.is_empty() || inner.starts_with('!') {
        return; // comment / doctype / stray `<>`
    }
    if let Some(rest) = inner.strip_prefix('/') {
        toks.push(Tok::Close(tag_name(rest)));
        return;
    }
    let name = tag_name(inner);
    if name == "br" {
        toks.push(Tok::Br);
        return;
    }
    let href = if name == "a" {
        extract_attr(inner, "href")
    } else {
        None
    };
    toks.push(Tok::Open { name, href });
}

/// The leading tag name, lowercased.
fn tag_name(s: &str) -> String {
    s.trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Extract a quoted attribute value (`name="..."` or `name='...'`).
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(attr) {
        let at = from + pos;
        // It's a *whole* attribute name only when both token boundaries are clean: preceded by space/start, AND
        // immediately (after optional spaces) followed by `=`. The after-check stops a name that is a PREFIX of a
        // longer one (`hreflang`, `data-href`) from matching `href`, its next char would be a name char, not `=`, so
        // the run is rejected and we leave the text literal.
        let ok_before = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let after = &tag[at + attr.len()..];
        let trimmed = after.trim_start();
        if ok_before && trimmed.starts_with('=') {
            let rest = trimmed[1..].trim_start();
            let value = if let Some(r) = rest.strip_prefix('"') {
                r.split('"').next().unwrap_or("")
            } else if let Some(r) = rest.strip_prefix('\'') {
                r.split('\'').next().unwrap_or("")
            } else {
                rest.split_whitespace().next().unwrap_or("")
            };
            return Some(decode_entities(value));
        }
        from = at + attr.len();
    }
    None
}

// ── Entity decoding ──────────────────────────────────────────────────────────

/// Longest entity body we'll scan for a terminating `;`, measured from the `&`.
/// Bounds the lookahead so a stray `&` in prose isn't treated as an entity, while
/// staying wide enough for the longest real refs: a hex code point with leading
/// zeros like `&#x0001F600;` (12 bytes incl. the `&`).
const MAX_ENTITY_LEN: usize = 12;

/// Decode the handful of HTML entities feeds actually use, plus numeric refs.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        // Only treat this as an entity if the `;` lands within a bounded window —
        // beyond that it's a literal `&` in prose, not a (very long) entity. `p` is
        // a byte offset of an ASCII `;`, so comparing it to a byte bound is safe.
        if let Some(semi) = tail.find(';').filter(|&p| p < MAX_ENTITY_LEN) {
            let entity = &tail[1..semi];
            let decoded = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => decode_numeric(entity),
            };
            if let Some(c) = decoded {
                out.push(c);
                rest = &tail[semi + 1..];
                continue;
            }
        }
        // Not a recognized entity — keep the '&' literally.
        out.push('&');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

fn decode_numeric(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix('#')?;
    let code = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
        u32::from_str_radix(hex, 16).ok()?
    } else {
        digits.parse::<u32>().ok()?
    };
    char::from_u32(code)
}

/// Collapse any run of ASCII whitespace into a single space (HTML semantics).
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

// ── Block parser ─────────────────────────────────────────────────────────────

/// Parse untrusted HTML into a safe block AST. Returns [`ParseError`] on
/// malformed input (e.g. an unterminated tag) so the caller can fall back to
/// rendering the raw string as escaped text.
pub fn parse(html: &str) -> Result<Vec<Block>, ParseError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<Inline> = Vec::new();
    let mut heading = false;
    let mut bold = 0usize;
    let mut link: Option<(String, String)> = None; // (href, accumulated text)
    let mut list: Option<Vec<Vec<Inline>>> = None;
    let mut li: Option<Vec<Inline>> = None;

    // Append an inline to whichever sink is active (list item or paragraph).
    fn sink<'a>(li: &'a mut Option<Vec<Inline>>, para: &'a mut Vec<Inline>) -> &'a mut Vec<Inline> {
        if let Some(item) = li.as_mut() {
            item
        } else {
            para
        }
    }

    let flush_para = |blocks: &mut Vec<Block>, para: &mut Vec<Inline>, heading: &mut bool| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph {
                inlines: std::mem::take(para),
                heading: *heading,
            });
        }
        *heading = false;
    };

    for tok in tokenize(html)? {
        match tok {
            Tok::Text(raw) => {
                let text = collapse_ws(&decode_entities(&raw));
                if text.trim().is_empty() {
                    // A whitespace-only run *between* inline elements still separates them (`<b>a</b> <b>b</b>` must
                    // render "a b", not "ab"). Preserve a single space, but only after existing content (never
                    // leading), and never doubling a trailing space/newline.
                    if !text.is_empty() {
                        if let Some((_, buf)) = link.as_mut() {
                            if !buf.is_empty() && !buf.ends_with([' ', '\n']) {
                                buf.push(' ');
                            }
                        } else {
                            let s = sink(&mut li, &mut para);
                            let needs = match s.last() {
                                Some(Inline::Text(t) | Inline::Bold(t)) => {
                                    !t.ends_with([' ', '\n'])
                                }
                                Some(Inline::Link { .. }) => true,
                                None => false,
                            };
                            if needs {
                                s.push(Inline::Text(" ".into()));
                            }
                        }
                    }
                    continue;
                }
                if let Some((_, buf)) = link.as_mut() {
                    buf.push_str(&text);
                } else if bold > 0 {
                    sink(&mut li, &mut para).push(Inline::Bold(text));
                } else {
                    sink(&mut li, &mut para).push(Inline::Text(text));
                }
            }
            Tok::Br => sink(&mut li, &mut para).push(Inline::Text("\n".into())),
            Tok::Open { name, href } => match name.as_str() {
                "p" | "div" => flush_para(&mut blocks, &mut para, &mut heading),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush_para(&mut blocks, &mut para, &mut heading);
                    heading = true;
                }
                "ul" | "ol" => {
                    flush_para(&mut blocks, &mut para, &mut heading);
                    list = Some(Vec::new());
                }
                "li" => li = Some(Vec::new()),
                "b" | "strong" => bold += 1,
                "a" => link = Some((href.unwrap_or_default(), String::new())),
                // i/em and any other whitelisted-but-styleless tag: keep text only.
                _ => {}
            },
            Tok::Close(name) => match name.as_str() {
                "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush_para(&mut blocks, &mut para, &mut heading)
                }
                "b" | "strong" => bold = bold.saturating_sub(1),
                "a" => {
                    if let Some((href, text)) = link.take() {
                        if text.is_empty() {
                            // nothing to show
                        } else if is_safe_href(&href) {
                            sink(&mut li, &mut para).push(Inline::Link { href, text });
                        } else {
                            sink(&mut li, &mut para).push(Inline::Text(text));
                        }
                    }
                }
                "li" => {
                    if let Some(item) = li.take() {
                        if let Some(l) = list.as_mut() {
                            if !item.is_empty() {
                                l.push(item);
                            }
                        } else if !item.is_empty() {
                            // <li> outside a list — fold into a paragraph.
                            para.extend(item);
                        }
                    }
                }
                "ul" | "ol" => {
                    if let Some(item) = li.take()
                        && let Some(l) = list.as_mut()
                        && !item.is_empty()
                    {
                        l.push(item);
                    }
                    if let Some(l) = list.take()
                        && !l.is_empty()
                    {
                        blocks.push(Block::List(l));
                    }
                }
                _ => {}
            },
        }
    }

    // Close anything left open.
    if let Some(item) = li.take() {
        if let Some(l) = list.as_mut() {
            l.push(item);
        } else {
            para.extend(item);
        }
    }
    if let Some(l) = list.take()
        && !l.is_empty()
    {
        blocks.push(Block::List(l));
    }
    flush_para(&mut blocks, &mut para, &mut heading);
    Ok(blocks)
}

/// Flatten untrusted HTML to a single plain-text string (tags stripped, entities
/// decoded, blocks separated by newlines). Used for compact previews. On a parse
/// error, returns the raw input (tags visible) per the fail-safe contract.
pub fn to_plain(html: &str) -> String {
    let Ok(blocks) = parse(html) else {
        return html.trim().to_string();
    };
    let mut lines: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph { inlines, .. } => lines.push(inline_text(&inlines)),
            Block::List(items) => {
                for item in items {
                    lines.push(format!("• {}", inline_text(&item)));
                }
            }
        }
    }
    lines.join("\n").trim().to_string()
}

fn inline_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Text(t) | Inline::Bold(t) => t.as_str(),
            Inline::Link { text, .. } => text.as_str(),
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_p_tags_to_paragraphs() {
        let blocks = parse("<p>First.</p><p>Second.</p>").unwrap();
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    inlines: vec![Inline::Text("First.".into())],
                    heading: false
                },
                Block::Paragraph {
                    inlines: vec![Inline::Text("Second.".into())],
                    heading: false
                },
            ]
        );
        assert_eq!(to_plain("<p>First.</p><p>Second.</p>"), "First.\nSecond.");
    }

    #[test]
    fn headings_become_bold_paragraphs() {
        let blocks = parse("<h2>Title</h2><p>Body</p>").unwrap();
        assert_eq!(
            blocks[0],
            Block::Paragraph {
                inlines: vec![Inline::Text("Title".into())],
                heading: true
            }
        );
    }

    #[test]
    fn lists_parse_items() {
        let blocks = parse("<ul><li>One</li><li>Two</li></ul>").unwrap();
        assert_eq!(
            blocks,
            vec![Block::List(vec![
                vec![Inline::Text("One".into())],
                vec![Inline::Text("Two".into())],
            ])]
        );
    }

    #[test]
    fn safe_links_kept_unsafe_demoted() {
        let safe = parse(r#"<p><a href="https://x.com">site</a></p>"#).unwrap();
        assert_eq!(
            safe[0],
            Block::Paragraph {
                inlines: vec![Inline::Link {
                    href: "https://x.com".into(),
                    text: "site".into()
                }],
                heading: false
            }
        );
        // javascript: scheme degrades to plain text — never a link.
        let unsafe_ = parse(r#"<p><a href="javascript:alert(1)">x</a></p>"#).unwrap();
        assert_eq!(
            unsafe_[0],
            Block::Paragraph {
                inlines: vec![Inline::Text("x".into())],
                heading: false
            }
        );
    }

    #[test]
    fn dangerous_tags_dropped_keeping_text() {
        // script/img/onclick are not whitelisted — only inner text survives, and
        // it is plain text (the UI escapes it on render).
        let blocks = parse(r#"<script>alert(1)</script><p onclick="evil()">hi</p>"#).unwrap();
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph {
                    inlines: vec![Inline::Text("alert(1)".into())],
                    heading: false
                },
                Block::Paragraph {
                    inlines: vec![Inline::Text("hi".into())],
                    heading: false
                },
            ]
        );
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#39;d&#39; &#x41;"),
            "a & b <c> 'd' A"
        );
    }

    #[test]
    fn decodes_long_numeric_ref() {
        // A hex code point with leading zeros (12 bytes incl. `&`) must still decode
        // — the bounded `;` lookahead reaches it instead of dropping it as prose.
        assert_eq!(decode_entities("emoji &#x0001F600; end"), "emoji 😀 end");
    }

    #[test]
    fn bold_runs() {
        let blocks = parse("<p>a <strong>b</strong> c</p>").unwrap();
        assert_eq!(
            blocks[0],
            Block::Paragraph {
                inlines: vec![
                    Inline::Text("a ".into()),
                    Inline::Bold("b".into()),
                    Inline::Text(" c".into()),
                ],
                heading: false
            }
        );
    }

    #[test]
    fn plain_text_without_tags_is_a_paragraph() {
        assert_eq!(to_plain("just text"), "just text");
    }

    #[test]
    fn unterminated_tag_is_a_parse_error() {
        // Fail-safe: malformed input errors so the UI shows the raw text.
        assert_eq!(parse("<p>oops <b broken"), Err(ParseError));
        assert_eq!(to_plain("<p>oops <b broken"), "<p>oops <b broken");
    }
}
