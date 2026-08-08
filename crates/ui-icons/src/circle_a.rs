use dioxus::prelude::*;

use super::outline_icon;

outline_icon!(
    /// Auto-advance **on** — a circled "A". Custom outline glyph drawn to match the
    /// Heroicons outline style used elsewhere (a ring + an "A": two legs + crossbar).
    CircleA,
    "0 0 24 24",
    "1.5",
    "M21 12A9 9 0 1 0 3 12 9 9 0 1 0 21 12M8.5 16 12 7l3.5 9M9.6 13h4.8"
);
