use dioxus::prelude::*;
use halogen_webui_commands::actions::subscribe_discovered;
use halogen_webui_hooks::{use_dispatch, use_podcasts, use_toast};
use halogen_wire::DiscoverResultItem;

#[component]
pub fn SubscribeButton(podcast: DiscoverResultItem, #[props(default)] disabled: bool) -> Element {
    let mut confirm = use_signal(|| false);
    let dispatch = use_dispatch();
    let toast = use_toast();
    let nav = use_navigator();
    let podcasts = use_podcasts();
    let subscribed = podcasts
        .read()
        .podcasts_by_id
        .values()
        .any(|p| p.feed_url == podcast.feed_url);
    rsx! {
        button { class: "btn btn-primary btn-sm", disabled: subscribed || disabled, onclick: move |_| confirm.set(true),
            if subscribed { "Subscribed" } else { "Subscribe" }
        }
        if confirm() {
            div { class: "modal modal-open",
                div { class: "modal-box", role: "dialog", "aria-modal": "true", "aria-labelledby": "discover-subscribe-title",
                    h2 { id: "discover-subscribe-title", class: "font-bold text-lg", "Subscribe to this podcast?" }
                    p { class: "py-3 break-words", "{podcast.title}" }
                    div { class: "modal-action",
                        button { class: "btn btn-ghost", onclick: move |_| confirm.set(false), "Cancel" }
                        button { class: "btn btn-primary",
                            onclick: move |_| {
                                subscribe_discovered(&dispatch, podcast.feed_url.clone(), podcast.title.clone(),
                                    (!podcast.description.is_empty()).then(|| podcast.description.clone()), podcast.author.clone());
                                confirm.set(false);
                                toast.success(format!("Subscribing to {}", podcast.title));
                                nav.push("/podcasts");
                            }, "Subscribe"
                        }
                    }
                }
                button { class: "modal-backdrop", "aria-label": "Cancel subscription", onclick: move |_| confirm.set(false) }
            }
        }
    }
}
