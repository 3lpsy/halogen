//! Cross-target persistence for the device log. native: NDJSON appended to `{data_dir}/halogen/device.log`. wasm:
//! IndexedDB `halogen.logs`, object store `logs` (auto-increment keys). Both keep at most [`super::CAP`] lines and
//! expose the same three async fns: [`load_all`] (oldest → newest, capped), [`append`], [`clear`].

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{append, clear, load_all};
#[cfg(target_arch = "wasm32")]
pub use web::{append, clear, load_all};
