use dioxus::prelude::*;

use super::outline_icon;

outline_icon!(
    /// Auto-advance **off** — a circled "A" struck through by a diagonal slash.
    /// Companion to [`super::CircleA`]; same ring + "A", plus the corner-to-corner
    /// strike that reads as "disabled".
    CircleAOff,
    "0 0 24 24",
    "1.5",
    "M21 12A9 9 0 1 0 3 12 9 9 0 1 0 21 12M8.5 16 12 7l3.5 9M9.6 13h4.8M5.6 5.6 18.4 18.4"
);
