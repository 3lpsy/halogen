// Intentional house style, not lint debt: `module_inception` is the
// `foo/foo.rs` + re-export layout used throughout, and `#[component]` functions
// legitimately take many props.
#![allow(clippy::module_inception, clippy::too_many_arguments)]
// Windows release builds: GUI subsystem, or every launch drags a console
// window along with the webview. Debug keeps the console (it's the only place
// stdout/tracing goes before the log file is set up). No-op off Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(test)]
mod tests;

// The UI is split across layered crates: `logging` + the services in the
// foundation/`svc-*` crates, `hooks`/providers/commands in `ui-state`, and the
// whole view layer (pages, routes, the `App` root) in `ui-views`. This binary is
// just the launch shim.
#[cfg(feature = "desktop")]
use dioxus::prelude::*;
use halogen_ui_logging as logging;
use halogen_ui_state::hooks;
use halogen_ui_views::App;

fn main() {
    logging::init();
    logging::info!("Halogen UI starting");
    register_service_worker();
    // Snapshot the URL query BEFORE the router mounts and normalizes it away, so a
    // deep-linked / shared list view (`/queue?sort=published.asc`) can seed from it.
    hooks::capture_initial_query();
    launch_app();
}

/// Web (and renderless test builds): plain launch — the platform config (title,
/// stylesheet, PWA head tags) comes from Dioxus.toml `[web.resource]` and the
/// custom `index.html` template.
#[cfg(not(feature = "desktop"))]
fn launch_app() {
    dioxus::launch(App);
}

/// Desktop: a configured window plus a stable webview data dir (cookies/storage
/// under our own data root — XDG-honoring, so flatpak lands it inside the
/// sandbox), launching the native shell.
#[cfg(feature = "desktop")]
fn launch_app() {
    use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
    let window = WindowBuilder::new()
        .with_title("Halogen")
        .with_inner_size(LogicalSize::new(1200.0, 800.0))
        .with_min_inner_size(LogicalSize::new(360.0, 640.0));
    let config = Config::new()
        .with_window(window)
        .with_data_directory(halogen_ui_platform::paths::data_root().join("webview"));
    // Linux/Windows: drop the default "Window"/"Edit" muda menu bar — in-window
    // clutter that does nothing useful here. macOS KEEPS the default menu: the
    // global menu bar's standard Edit items are what route Cmd+C/V/X into the
    // webview — removing the menu silently breaks copy/paste in every input.
    #[cfg(not(target_os = "macos"))]
    let config = config.with_menu(None);
    dioxus::LaunchBuilder::new()
        .with_cfg(config)
        .launch(NativeApp);
}

/// Native root: `App` plus the head bits the web build gets from the
/// Dioxus.toml `[web.resource]` block — that block only applies to the web
/// platform, so without this the native webview renders completely unstyled.
/// The stylesheet is EMBEDDED (`include_str!`, inline `<style>`), not an
/// `asset!()` link: the released binaries (bare ELF, windows zip, flatpak,
/// AppImage) must run standalone, never depending on an
/// `assets/` dir existing next to the exe — same philosophy as the server's
/// embed-frontend. include_str reads crates/ui/assets/tailwind.css at COMPILE
/// time, which `just tailwind` produces — always build native through the
/// `just ui-*` recipes (a placeholder from `_tailwind-ensure` bakes in
/// unstyled output).
#[cfg(feature = "desktop")]
#[component]
fn NativeApp() -> Element {
    rsx! {
        document::Style { {include_str!("../assets/tailwind.css")} }
        App {}
    }
}

/// Register the PWA service worker (`/sw.js`, served from the dist root so its
/// scope is `/`). wasm-only; a no-op on native builds. Fire-and-forget — the
/// returned Promise is dropped, and the app works fine if registration fails.
#[cfg(target_arch = "wasm32")]
fn register_service_worker() {
    if let Some(win) = web_sys::window() {
        let _ = win.navigator().service_worker().register("/sw.js");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn register_service_worker() {}
