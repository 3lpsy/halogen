//! SVG components use Font Awesome Free, CC BY 4.0 <https://fontawesome.com/license/free>, and Heroicons, MIT
//! <https://github.com/tailwindlabs/heroicons>; each file retains attribution. Filled/outline macros generate standard
//! icons, custom bodies remain explicit. Pass Tailwind classes via class; unused shared exports are allowed.
#![allow(unused_imports)]

/// Generate a filled icon component (Font-Awesome solid style): a single `currentColor` path inside an `svg`. Expands
/// to `#[component] pub fn $name(class: &'static str) -> Element`. The per-icon file must `use dioxus::prelude::*;`
/// (the `#[component]` expansion needs `dioxus_core` in scope), exactly as the hand-written icons did.
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
