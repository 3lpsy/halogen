//! Add a podcast by feed URL. Online-only.
//!
//! Reached from the Discover page's "Add by URL" action (moved here from the
//! Settings page). Validates the URL client-side, then subscribes through the
//! worker — the same canonical `Command::Subscribe` path used everywhere else.

use dioxus::prelude::*;

use crate::Route;
use crate::components::BackButton;
use halogen_ui_state::commands;
use halogen_ui_state::hooks::{use_dispatch, use_sync_status, use_toast};

/// Validate a user-entered feed URL: non-empty, parseable, http(s) scheme.
/// Returns the normalized URL string, or a message to show inline.
fn validate_feed_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Enter a feed URL.".to_string());
    }
    match url::Url::parse(raw) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => Ok(u.to_string()),
        Ok(_) => Err("Feed URL must start with http:// or https://.".to_string()),
        Err(_) => Err("That doesn't look like a valid URL.".to_string()),
    }
}

#[component]
pub fn PodcastCreate() -> Element {
    let dispatch = use_dispatch();
    let toast = use_toast();
    let sync_status = use_sync_status();
    let nav = use_navigator();

    let mut feed_url = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);

    let offline = sync_status().is_offline();

    rsx! {
        div { class: "h-full overflow-y-auto",
        div { class: "p-2 max-w-xl mx-auto space-y-4",
            BackButton {}
            h1 { class: "text-2xl font-bold", "Add a podcast" }
            p { class: "text-sm text-muted",
                "Subscribe directly by RSS feed URL. The server fetches the feed and ingests its episodes."
            }

            if offline {
                div { class: "alert alert-warning text-sm",
                    "Adding a podcast needs an internet connection." }
            }

            form {
                class: "space-y-3",
                onsubmit: move |e| {
                    e.prevent_default();
                    if offline {
                        return;
                    }
                    match validate_feed_url(&feed_url.peek()) {
                        Ok(url) => {
                            error.set(None);
                            commands::subscribe(&dispatch, url);
                            toast.success("Subscribing…");
                            nav.push(Route::Podcasts {});
                        }
                        Err(msg) => error.set(Some(msg)),
                    }
                },
                div {
                    label { class: "label", span { class: "label-text", "Feed URL" } }
                    input {
                        "aria-label": "Feed URL",
                        r#type: "url",
                        placeholder: "https://feed.example.com/rss",
                        class: if error().is_some() {
                            "input input-bordered input-error w-full"
                        } else {
                            "input input-bordered w-full"
                        },
                        value: "{feed_url}",
                        disabled: offline,
                        // Clear the inline error as the user edits.
                        oninput: move |e| {
                            feed_url.set(e.value());
                            error.set(None);
                        },
                    }
                    if let Some(msg) = error() {
                        p { class: "text-error text-sm mt-1", "{msg}" }
                    }
                }
                div { class: "flex gap-2",
                    button {
                        r#type: "submit",
                        class: "btn btn-primary",
                        disabled: offline,
                        "Subscribe"
                    }
                    button {
                        r#type: "button",
                        class: "btn btn-ghost",
                        onclick: move |_| {
                            nav.push(Route::Discover {});
                        },
                        "Cancel"
                    }
                }
            }
        }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_feed_url;

    #[test]
    fn accepts_http_and_https() {
        assert!(validate_feed_url("https://example.com/feed.xml").is_ok());
        assert!(validate_feed_url("http://example.com/feed.xml").is_ok());
        // Surrounding whitespace is trimmed.
        assert_eq!(
            validate_feed_url("  https://example.com/feed.xml  ").unwrap(),
            "https://example.com/feed.xml"
        );
    }

    #[test]
    fn rejects_empty_bad_scheme_and_garbage() {
        assert!(validate_feed_url("   ").is_err());
        assert!(validate_feed_url("ftp://example.com/feed.xml").is_err());
        assert!(validate_feed_url("not a url").is_err());
    }
}
