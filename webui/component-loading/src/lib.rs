use dioxus::prelude::*;
/// Boot splash shown by a provider while its async load resolves, so downstream
/// guards never see a transient default. The one shared copy used by
/// `AccountsProvider` and `ConfigProvider`.
#[component]
pub fn LoadingSplash() -> Element {
    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-background",
            div { class: "text-muted", "Loading…" }
        }
    }
}
