//! Crate-level test modules (compiled only under `cfg(test)`, native). Component-local unit tests stay next to their
//! code; what lives here is the cross-cutting suites, currently the Dioxus state/re-render semantics the app's state
//! architecture depends on.

mod render_semantics;
