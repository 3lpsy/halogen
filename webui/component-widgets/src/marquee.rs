use dioxus::prelude::*;

/// Threshold (in characters) above which text is assumed to overflow its box and
/// is scrolled; at or below it the text fits, so it's left static.
const SCROLL_THRESHOLD: usize = 24;

/// Scroll overflowing text at a length-scaled speed after an initial pause; fitting text stays still. Two padded copies
/// translate by half the track for seamless looping; keyframes live in tailwind.css and callers supply text/container
/// classes.
#[component]
pub fn Marquee(
    text: String,
    /// Extra classes for the clipping container (sizing / typography).
    #[props(default)]
    class: String,
    /// Force the scroll even when the text would fit AND even under
    /// `prefers-reduced-motion` (which the `.animate-marquee` class disables). The
    /// animation is applied inline so it wins over the media-query rule — used by
    /// `UpNext`, where tapping the labels opts into a scroll on demand.
    #[props(default)]
    force: bool,
) -> Element {
    let len = text.chars().count();
    if len <= SCROLL_THRESHOLD && !force {
        // Fits: no animation. `truncate` is a safety net if the box is narrower
        // than the heuristic assumed. `pointer-events-none` (see the scrolling
        // branch) keeps taps falling through to an enclosing button.
        return rsx! {
            div { class: "truncate pointer-events-none {class}", "{text}" }
        };
    }
    // Constant scroll speed (~3 chars/sec) with a 16s floor so shorter overflows
    // don't whip past. Two copies, so the travelled distance is one copy + gap.
    let duration = (len as f32 / 3.0).max(16.0);
    // Forced: drive the animation INLINE so it runs even under
    // `prefers-reduced-motion` (the `.animate-marquee` class is reset to
    // `animation: none` there) and for short text. Otherwise drive it from the
    // class, which honors reduced motion.
    let (track_class, track_style) = if force {
        (
            "flex w-max",
            format!("animation: marquee {duration}s linear infinite 2s;"),
        )
    } else {
        (
            "flex w-max animate-marquee",
            format!("animation-duration: {duration}s;"),
        )
    };
    rsx! {
        // `pointer-events-none`: the track is transform-animated, and iOS Safari drops taps that land on an actively
        // animated layer (hit-test uses the layout box, not the live composited position). Making the marquee
        // non-interactive lets taps fall through to an enclosing button (e.g. `UpNext`), so navigation fires reliably
        // instead of intermittently.
        div { class: "overflow-hidden pointer-events-none {class}",
            div {
                class: "{track_class}",
                style: "{track_style}",
                span { class: "whitespace-nowrap pr-8", "{text}" }
                // Second copy makes the wrap seamless; hidden from the a11y tree.
                span { class: "whitespace-nowrap pr-8", "aria-hidden": "true", "{text}" }
            }
        }
    }
}
