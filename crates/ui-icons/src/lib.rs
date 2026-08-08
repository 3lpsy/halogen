//! Reusable SVG icon components.
//!
//! Each icon is its own file holding just the `svg`. Most are from **Font Awesome
//! Free** (solid, CC BY 4.0 — <https://fontawesome.com/license/free>); a few are
//! **Heroicons** (outline, MIT — <https://github.com/tailwindlabs/heroicons>). Each
//! file names its source. Pass a Tailwind size/colour class via the `class` prop,
//! e.g. `Play { class: "w-4 h-4" }`.
//!
//! This is a shared icon library: not every icon is wired up at all times (one may
//! be swapped out of a component but kept for later), so unused re-exports are fine.
//!
//! Most icons are pure boilerplate that differs only in name / `view_box` / path,
//! so two declarative macros generate the component. [`icon!`] covers the filled
//! Font-Awesome family (`fill: currentColor`, one `path`); [`outline_icon!`] covers
//! the Heroicons-style outline family (`fill: none` + stroke). Each per-icon file is
//! just one macro call plus its source-attribution doc comment; a single icon with
//! a non-standard body (`circle_half`, which needs two paths) is still hand-written.
#![allow(unused_imports)]

/// Generate a filled icon component (Font-Awesome solid style): a single
/// `currentColor` path inside an `svg`. Expands to
/// `#[component] pub fn $name(class: &'static str) -> Element`. The per-icon file
/// must `use dioxus::prelude::*;` (the `#[component]` expansion needs `dioxus_core`
/// in scope), exactly as the hand-written icons did.
macro_rules! icon {
    ($(#[$doc:meta])* $name:ident, $view_box:literal, $path:literal) => {
        $(#[$doc])*
        #[dioxus::prelude::component]
        pub fn $name(class: &'static str) -> dioxus::prelude::Element {
            dioxus::prelude::rsx! {
                svg {
                    xmlns: "http://www.w3.org/2000/svg",
                    class: "{class}",
                    fill: "currentColor",
                    view_box: $view_box,
                    path { d: $path }
                }
            }
        }
    };
}

/// Generate an outline icon component (Heroicons style): `fill: none`, a stroke of
/// `$stroke_width`, and a single round-capped/joined `currentColor` stroke path.
/// Same `use dioxus::prelude::*;` requirement as [`icon!`].
macro_rules! outline_icon {
    ($(#[$doc:meta])* $name:ident, $view_box:literal, $stroke_width:literal, $path:literal) => {
        $(#[$doc])*
        #[dioxus::prelude::component]
        pub fn $name(class: &'static str) -> dioxus::prelude::Element {
            dioxus::prelude::rsx! {
                svg {
                    xmlns: "http://www.w3.org/2000/svg",
                    class: "{class}",
                    fill: "none",
                    view_box: $view_box,
                    stroke_width: $stroke_width,
                    stroke: "currentColor",
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: $path,
                    }
                }
            }
        }
    };
}

// Make the macros importable by the per-icon submodules via `use super::icon;`.
pub(crate) use {icon, outline_icon};

mod arrow_path;
mod arrows_up_down;
mod backward_step;
mod bars;
mod bolt;
mod chevron_down;
mod chevron_left;
mod chevron_right;
mod circle_a;
mod circle_a_off;
mod circle_check;
mod circle_half;
mod clock_rotate_left;
mod cloud_arrow_down;
mod document_text;
mod download;
mod face_frown;
mod forward_step;
mod funnel;
mod gear;
mod globe_alt;
mod grip_vertical;
mod list_ul;
mod magnifying_glass;
mod moon;
mod music;
mod pause;
mod pencil;
mod play;
mod plus;
mod podcast;
mod shield_halved;
mod trash;
mod user;
mod viewfinder_circle;
mod xmark;

pub use arrow_path::ArrowPath;
pub use arrows_up_down::ArrowsUpDown;
pub use backward_step::BackwardStep;
pub use bars::Bars;
pub use bolt::Bolt;
pub use chevron_down::ChevronDown;
pub use chevron_left::ChevronLeft;
pub use chevron_right::ChevronRight;
pub use circle_a::CircleA;
pub use circle_a_off::CircleAOff;
pub use circle_check::CircleCheck;
pub use circle_half::CircleHalf;
pub use clock_rotate_left::ClockRotateLeft;
pub use cloud_arrow_down::CloudArrowDown;
pub use document_text::DocumentText;
pub use download::Download;
pub use face_frown::FaceFrown;
pub use forward_step::ForwardStep;
pub use funnel::Funnel;
pub use gear::Gear;
pub use globe_alt::GlobeAlt;
pub use grip_vertical::GripVertical;
pub use list_ul::ListUl;
pub use magnifying_glass::MagnifyingGlass;
pub use moon::Moon;
pub use music::Music;
pub use pause::Pause;
pub use pencil::Pencil;
pub use play::Play;
pub use plus::Plus;
pub use podcast::Podcast;
pub use shield_halved::ShieldHalved;
pub use trash::Trash;
pub use user::User;
pub use viewfinder_circle::ViewfinderCircle;
pub use xmark::XMark;
