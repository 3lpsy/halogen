//! Safe renderer for untrusted feed HTML (episode/podcast descriptions).
//!
//! [`RichText`] parses the HTML into a whitelisted AST ([`crate::html`])
//! and renders it through ordinary `rsx!` — text is escaped, only `p`/headings/
//! lists/bold and vetted links survive. Links are NOT navigated directly: they
//! call back so the page can confirm via [`ConfirmLinkModal`] first. On a parse
//! error the raw string is shown as escaped text (tags visible).

use dioxus::prelude::*;

use crate::ConfirmModal;
use crate::html::{Block, Inline, parse};

/// Render untrusted HTML safely. `on_link` fires with the (already scheme-vetted)
/// href when a link is tapped — wire it to a [`ConfirmLinkModal`].
#[component]
pub fn RichText(html: String, on_link: Callback<String>) -> Element {
    match parse(&html) {
        // `break-words` (overflow-wrap is inherited) binds long URLs/tokens to the
        // container width so the description wraps instead of forcing horizontal
        // scroll. `max-w-full min-w-0` keeps it from outgrowing a flex parent.
        Ok(blocks) if !blocks.is_empty() => rsx! {
            div { class: "space-y-2 break-words max-w-full min-w-0",
                for block in blocks {
                    {render_block(block, on_link)}
                }
            }
        },
        // Fail-safe: show the raw input as escaped text (tags visible).
        _ => rsx! {
            p { class: "whitespace-pre-line break-words max-w-full min-w-0", "{html}" }
        },
    }
}

fn render_block(block: Block, on_link: Callback<String>) -> Element {
    match block {
        Block::Paragraph { inlines, heading } => {
            let class = if heading {
                "font-bold whitespace-pre-line"
            } else {
                "whitespace-pre-line"
            };
            rsx! {
                p { class: "{class}",
                    for inl in inlines {
                        {render_inline(inl, on_link)}
                    }
                }
            }
        }
        Block::List(items) => rsx! {
            ul { class: "list-disc list-inside space-y-1",
                for item in items {
                    li {
                        for inl in item {
                            {render_inline(inl, on_link)}
                        }
                    }
                }
            }
        },
    }
}

fn render_inline(inline: Inline, on_link: Callback<String>) -> Element {
    match inline {
        Inline::Text(t) => rsx! { "{t}" },
        Inline::Bold(t) => rsx! { b { "{t}" } },
        Inline::Link { href, text } => rsx! {
            button {
                class: "link link-primary",
                onclick: move |_| on_link.call(href.clone()),
                "{text}"
            }
        },
    }
}

/// Confirmation dialog for opening an external link. Driven by a signal holding
/// the pending href; `Some` shows the modal, `Open` is a real anchor (new tab,
/// `noopener`), `Cancel`/backdrop clear it.
#[component]
pub fn ConfirmLinkModal(pending: Signal<Option<String>>) -> Element {
    let mut pending = pending;
    let current = pending.read().clone();
    rsx! {
        if let Some(href) = current {
            ConfirmModal {
                title: "Open external link?",
                body: href.clone(),
                note: "This link comes from the podcast feed and opens in a new tab.",
                confirm_label: "Open",
                confirm_href: href.clone(),
                title_id: "confirm-external-link-title",
                on_cancel: move |_| pending.set(None),
                on_confirm: move |_| pending.set(None),
            }
        }
    }
}
