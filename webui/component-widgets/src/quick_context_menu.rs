//! Root-hosted menus escape transformed row clipping and may outlive their callers, so actions use scope-independent Rc
//! closures rather than scoped Callbacks. Anchor frequent actions near the bottom on mobile and the top on desktop. See
//! the crate README for usage.

use std::rc::Rc;

use dioxus::prelude::*;

use halogen_webui_component_icons::{
    ArrowPath, ArrowsUpDown, BackwardStep, Bars, ChevronDown, CloudArrowDown, DocumentText,
    Download, ForwardStep, Gear, ListUl, Music, Pencil, Play, Podcast, Trash, XMark,
};

/// Leading icon for a [`QuickAction`], rendered to the left of its label.
#[derive(Clone, Copy, PartialEq)]
pub enum QuickIcon {
    /// Stream / play.
    Play,
    /// Download to this device.
    Download,
    /// Download on the server (cloud).
    CloudDownload,
    /// Remove / delete.
    Trash,
    /// View podcast.
    Podcast,
    /// Settings / configuration (gear).
    Settings,
    /// View episode.
    Episode,
    /// Add to / from a playlist (queue).
    Queue,
    /// Add to one or more named playlists (opens the playlist picker).
    AddPlaylist,
    /// Reorder: move up one.
    MoveUp,
    /// Reorder: move down one.
    MoveDown,
    /// Reorder: move to the start.
    MoveFirst,
    /// Reorder: move to the end.
    MoveLast,
    /// Smart reorder (sort) — the up/down arrows glyph.
    Reorder,
    /// Recover / reset local state — the circular-arrow glyph. Used by the
    /// "Remove local data" actions (distinct from `Trash`'s server/device removes).
    Reset,
    /// View raw metadata — the document glyph.
    Metadata,
    /// Edit the entity's own fields (pencil), distinct from `Settings`'s
    /// gear (which configures behavior, not data).
    Edit,
}

impl QuickIcon {
    fn render(self) -> Element {
        let class = "w-4 h-4 shrink-0";
        match self {
            QuickIcon::Play => rsx! { Play { class } },
            QuickIcon::Download => rsx! { Download { class } },
            QuickIcon::CloudDownload => rsx! { CloudArrowDown { class } },
            QuickIcon::Trash => rsx! { Trash { class } },
            QuickIcon::Podcast => rsx! { Podcast { class } },
            QuickIcon::Settings => rsx! { Gear { class } },
            QuickIcon::Episode => rsx! { Music { class } },
            QuickIcon::Queue => rsx! { ListUl { class } },
            QuickIcon::AddPlaylist => rsx! { Bars { class } },
            // A chevron rotated 180° serves as the "up" arrow (no separate SVG).
            QuickIcon::MoveUp => rsx! { ChevronDown { class: "w-4 h-4 shrink-0 rotate-180" } },
            QuickIcon::MoveDown => rsx! { ChevronDown { class } },
            QuickIcon::MoveFirst => rsx! { BackwardStep { class } },
            QuickIcon::MoveLast => rsx! { ForwardStep { class } },
            QuickIcon::Reorder => rsx! { ArrowsUpDown { class } },
            QuickIcon::Reset => rsx! { ArrowPath { class } },
            QuickIcon::Metadata => rsx! { DocumentText { class } },
            QuickIcon::Edit => rsx! { Pencil { class } },
        }
    }
}

/// One selectable row in the menu.
#[derive(Clone)]
pub struct QuickAction {
    pub label: String,
    /// Leading icon shown to the left of the label.
    pub icon: QuickIcon,
    /// Scope-independent select handler — see the module docs for why this is an
    /// `Rc` closure and must never wrap a scope-owned `Callback`.
    pub on_select: Rc<dyn Fn()>,
    /// Greyed out + non-actionable (e.g. a reorder action when the list isn't in
    /// Custom order, or a boundary move like "up" on the first row).
    pub disabled: bool,
}

impl QuickAction {
    /// An enabled action. Chain [`QuickAction::disabled`] to grey it out.
    /// `on_select` should capture only scope-independent `Copy` handles
    /// (`Coroutine<Command>`, `Signal<…>`, `Navigator`) — see the module docs.
    pub fn new(label: impl Into<String>, icon: QuickIcon, on_select: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            icon,
            on_select: Rc::new(on_select),
            disabled: false,
        }
    }

    /// Builder: set the `disabled` flag (greyed out + non-actionable).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Backing state for the single menu instance.
#[derive(Clone, Default)]
pub struct QuickMenuState {
    pub open: bool,
    pub title: Option<String>,
    /// Action groups; a divider line renders between consecutive sections.
    pub sections: Vec<Vec<QuickAction>>,
}

/// Context handle for opening/closing the shared menu. `Copy`, so it can be
/// grabbed via [`use_quick_menu`] and used inside event handlers freely.
#[derive(Clone, Copy)]
pub struct QuickMenu(Signal<QuickMenuState>);

impl QuickMenu {
    pub fn new() -> Self {
        Self(Signal::new(QuickMenuState::default()))
    }

    /// Open the menu with the given title and action sections.
    pub fn open(&self, title: String, sections: Vec<Vec<QuickAction>>) {
        let mut state = self.0;
        state.set(QuickMenuState {
            open: true,
            title: Some(title),
            sections,
        });
    }

    /// Close the menu (keeps the last actions; just hides the panel).
    pub fn close(&self) {
        let mut state = self.0;
        state.write().open = false;
    }
}

impl Default for QuickMenu {
    fn default() -> Self {
        Self::new()
    }
}

/// Read the shared [`QuickMenu`] controller from context.
pub fn use_quick_menu() -> QuickMenu {
    use_context::<QuickMenu>()
}

/// The single overlay host. Render once near the app root (it provides nothing —
/// the [`QuickMenu`] context must already be provided by the parent). Always
/// mounted so the panel can slide in *and* out.
#[component]
pub fn QuickContextMenuHost() -> Element {
    let quick = use_quick_menu();
    let QuickMenu(state) = quick;
    let snap = state.read().clone();
    let open = snap.open;

    // Full literal class strings per state so Tailwind's source scan emits both.
    let container_class = if open {
        "fixed inset-0 z-[60]"
    } else {
        "fixed inset-0 z-[60] pointer-events-none"
    };
    let backdrop_class = if open {
        "absolute inset-0 bg-black/40 transition-opacity duration-200 opacity-100"
    } else {
        "absolute inset-0 bg-black/40 transition-opacity duration-200 opacity-0"
    };
    let panel_class = if open {
        "absolute top-0 right-0 bottom-0 w-80 max-w-[85%] bg-base-100 shadow-xl flex flex-col md:flex-col-reverse transition-transform duration-200 ease-out pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] translate-x-0"
    } else {
        "absolute top-0 right-0 bottom-0 w-80 max-w-[85%] bg-base-100 shadow-xl flex flex-col md:flex-col-reverse transition-transform duration-200 ease-out pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] translate-x-full"
    };

    rsx! {
        div { class: "{container_class}",
            // Backdrop — tap to dismiss.
            div {
                class: "{backdrop_class}",
                onclick: move |_| quick.close(),
            }
            // Right sidecar panel. Mobile: bottom-anchored — a flex spacer pushes
            // the sections down so the last section sits just above the header.
            // Desktop (`md:flex-col-reverse`): mirrored to top-anchored, header
            // first, sections below it (the spacer then pushes them up to it).
            aside { class: "{panel_class}",
                // Reverse mobile flex scrolling so overflow hides upper, less-used actions while keeping bottom actions
                // reachable. Render reversed sections to retain authored visual order; desktop uses normal top-anchored
                // scrolling.
                div { class: "flex-1 overflow-y-auto p-2 flex flex-col-reverse md:flex-col",
                    for (i, section) in snap.sections.iter().filter(|s| !s.is_empty()).rev().enumerate() {
                        if i > 0 {
                            div { class: "border-t border-base-300 my-2" }
                        }
                        // Each section is its own column so reversing the parent
                        // mirrors only the SECTION order — the buttons within a
                        // section keep their authored order on both.
                        div { class: "flex flex-col",
                            for action in section.iter() {
                                button {
                                    class: if action.disabled {
                                        "btn btn-ghost btn-block justify-start gap-3 btn-disabled opacity-40"
                                    } else {
                                        "btn btn-ghost btn-block justify-start gap-3"
                                    },
                                    disabled: action.disabled,
                                    onclick: {
                                        let cb = action.on_select.clone();
                                        let dis = action.disabled;
                                        move |_| {
                                            if !dis {
                                                cb();
                                                quick.close();
                                            }
                                        }
                                    },
                                    {action.icon.render()}
                                    "{action.label}"
                                }
                            }
                        }
                    }
                }
                // Header: close (left) + episode title. Mobile sits at the bottom
                // (divider above it); desktop sits at the top (divider below it).
                div { class: "flex items-center gap-2 p-3 border-t md:border-t-0 md:border-b border-base-200",
                    button {
                        "aria-label": "Close menu",
                        class: "btn btn-ghost btn-square",
                        onclick: move |_| quick.close(),
                        XMark { class: "w-5 h-5" }
                    }
                    if let Some(title) = snap.title.clone() {
                        // Not a heading — this overlay's title has no h1/h2 above it
                        // (would skip levels for Lighthouse). Same look as the old h3.
                        div { class: "font-semibold text-sm truncate flex-1", "{title}" }
                    }
                }
            }
        }
    }
}
