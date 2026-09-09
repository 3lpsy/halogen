use dioxus::prelude::*;

/// `/` has no standalone screen — it redirects to the Queue. Kept as a thin
/// component so the route resolves; it renders nothing before the redirect
/// commits.
#[component]
pub fn Home() -> Element {
    let nav = use_navigator();
    use_effect(move || {
        nav.replace("/queue");
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_component_creates() {
        let mut vdom = VirtualDom::new(|| rsx! { Home {} });
        vdom.rebuild(&mut dioxus::core::NoOpMutations);
    }
}
