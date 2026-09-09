//! Request artwork optimistically from server art endpoints, never feed origins. The server resolves/cache-falls-back
//! lazily; onerror replaces failed requests with a placeholder rather than a broken image.

use dioxus::prelude::*;

use halogen_webui_component_icons::FaceFrown;
use halogen_webui_hooks::use_sync_status;

/// An `<img>` for a server art URL, falling back to a centred frown glyph when
/// the source is absent or the request fails.
#[component]
pub fn Artwork(
    /// Server art endpoint URL. `None` = nothing to request (no session yet); renders the placeholder immediately.
    /// `ReadSignal` so a changed URL (e.g. detail page navigating between ids without a remount) resets the failure
    /// latch via the effect below.
    src: ReadSignal<Option<String>>,
    alt: String,
    /// Optionally paint the cached small thumbnail as a background while full artwork loads. ReadSignal tracks
    /// detail-ID changes without remounting; None keeps normal single-source rendering.
    #[props(default)]
    placeholder_src: ReadSignal<Option<String>>,
    /// Classes for the `<img>` (the parent renders the framing container).
    img_class: String,
    /// Classes for the fallback container (sizing / extra styling). The frown
    /// glyph inside it scales to the container and carries its own colour.
    #[props(default)]
    placeholder_class: String,
    /// Optional intrinsic pixel dimensions for the `<img>`. When set, they become the `width`/`height` HTML attributes
    /// so the browser reserves space from the derived aspect ratio BEFORE the bytes load, preventing layout shift (CLS)
    /// even though the rendered size still comes from `img_class`. Pass the natural art aspect (e.g. a square
    /// `Some(512)`/`Some(512)`); the CSS sizing wins.
    #[props(default)]
    width: Option<u32>,
    #[props(default)] height: Option<u32>,
) -> Element {
    let mut failed = use_signal(|| false);
    // Whether the full `src` has painted. Until then, a `placeholder_src` (if any)
    // shows through as the `<img>` background. Only meaningful when a placeholder
    // is set — list/mini tiles leave it untouched (their `onload` is a no-op).
    let mut loaded = use_signal(|| false);

    // Coarse connectivity flag, NOT the full sync status: a `PartialEq`-gated memo
    // that only flips on an actual offline↔online transition, so the periodic
    // Online→Syncing→Online pull cycle doesn't re-render every row's art.
    let sync_status = use_sync_status();
    let offline = use_memo(move || sync_status().is_offline());

    // Clear the failure latch on source or connectivity changes so recovered artwork retries without remounting. Equal
    // source URLs remain value-gated.
    use_effect(move || {
        let _ = src.read();
        let _ = offline();
        // Value-gate: `Signal::set` notifies unconditionally, so only write when the
        // latch is actually set — avoids re-rendering every art tile on each
        // (equal) source/connectivity tick.
        if *failed.peek() {
            failed.set(false);
        }
        // New source ⇒ the full image must re-load, so re-show the placeholder.
        if *loaded.peek() {
            loaded.set(false);
        }
    });

    let current = src.read().clone();
    let placeholder = placeholder_src.read().clone();
    let has_placeholder = placeholder.is_some();
    match current {
        Some(url) if !failed() => {
            // Paint the small thumbnail behind the full image until it loads. `cover`
            // matches the common `object-cover` tiles; same-source aspect makes the
            // brief mismatch under `object-contain` (full player) imperceptible.
            let bg_style = match (&placeholder, loaded()) {
                (Some(p), false) => format!(
                    "background-image:url(\"{p}\");background-size:cover;\
                     background-position:center;background-repeat:no-repeat;"
                ),
                _ => String::new(),
            };
            rsx! {
                img {
                    src: "{url}",
                    class: "{img_class}",
                    style: "{bg_style}",
                    alt: "{alt}",
                    // Intrinsic dimensions (omitted when None) give the browser an
                    // aspect ratio to reserve space pre-load — CSS classes still own
                    // the rendered size. Keeps art tiles from shifting layout on load.
                    width: width,
                    height: height,
                    loading: "lazy",
                    // Send media cookies only on web. Native uses nonce-protected bridge URLs without credentialed CORS
                    // so img and CSS background loads share a cache mode instead of failing against cached responses
                    // after navigation.
                    crossorigin: cfg!(target_arch = "wasm32").then_some("use-credentials"),
                    // Drop the placeholder background once the full image paints.
                    // Gated on `has_placeholder` so plain tiles never re-render here.
                    onload: move |_| {
                        if has_placeholder && !*loaded.peek() {
                            loaded.set(true);
                        }
                    },
                    onerror: move |_| failed.set(true),
                }
            }
        }
        // No art (no URL yet, a 404, or a network error): a centred frown glyph.
        // `text-muted` keeps it ≥3:1 (graphical-object contrast) and self-coloured
        // so a caller's faint `placeholder_class` text colour can't wash it out;
        // `w-1/2` scales it to any tile size (mini player → full-screen art).
        _ => rsx! {
            div {
                class: "w-full h-full flex items-center justify-center {placeholder_class}",
                FaceFrown { class: "w-1/2 h-1/2 max-w-16 max-h-16 text-muted" }
            }
        },
    }
}
