use dioxus::prelude::*;

/// Half-filled circle — the "played but not finished" (in-progress) playback
/// marker, paired with the solid `circle-check` shown once an episode is finished.
/// An outline ring (first path) with the right semicircle filled (second path);
/// both use `currentColor`.
#[component]
pub fn CircleHalf(class: &'static str) -> Element {
    rsx! {
        svg {
            xmlns: "http://www.w3.org/2000/svg",
            class: "{class}",
            fill: "none",
            view_box: "0 0 24 24",
            // Outline ring — full circle drawn as two half-arcs.
            path {
                d: "M12 3 A9 9 0 1 0 12 21 A9 9 0 1 0 12 3 Z",
                stroke: "currentColor",
                stroke_width: "2",
            }
            // Filled right half — top to bottom arc, closed back up the diameter.
            path { d: "M12 3 A9 9 0 0 1 12 21 Z", fill: "currentColor" }
        }
    }
}
