//! `resource_view` — the shared Loading/Error rendering for `use_resource` reads.

use dioxus::prelude::*;

/// Render the loading + error states of a `use_resource` value uniformly and defer to `ok` for the loaded data. `state`
/// is the dereferenced resource read (`&*resource.read_unchecked()`); `noun` fills the error line, "Could not load
/// {noun}: …". The empty-vs-content split (where one exists) stays in the caller's `ok` closure, since "empty" is
/// collection-specific.
pub fn resource_view<T>(
    state: &Option<Result<T, String>>,
    noun: &str,
    ok: impl FnOnce(&T) -> Element,
) -> Element {
    match state {
        None => rsx! {
            p { class: "text-muted", "Loading…" }
        },
        Some(Err(err)) => rsx! {
            div { role: "alert", class: "alert alert-error", "Could not load {noun}: {err}" }
        },
        Some(Ok(value)) => ok(value),
    }
}

/// [`resource_view`] specialized for a `Vec<T>`: render loading/error uniformly, an `empty` message when the list is
/// empty, else `row` over each item inside a `container_class` flex column. Folds the `if list.is_empty() {…} else {…}`
/// split every list-shaped `use_resource` page otherwise repeats. `row` returns the keyed element for one item.
pub fn resource_list_view<T: Clone>(
    state: &Option<Result<Vec<T>, String>>,
    noun: &str,
    empty: &str,
    container_class: &str,
    row: impl Fn(T) -> Element,
) -> Element {
    resource_view(state, noun, |list: &Vec<T>| {
        if list.is_empty() {
            rsx! {
                p { class: "text-muted", "{empty}" }
            }
        } else {
            rsx! {
                div { class: "{container_class}",
                    for item in list.clone() {
                        {row(item)}
                    }
                }
            }
        }
    })
}
