//! Halogen API client — a typed gateway to the Axum backend. Callers pass a `*Data` / `*Params` value and get
//! back unwrapped `*Data` (plus `Paginator` for lists), or a typed `ApiError`. No touching of `RequestData` /
//! `ResponseData` envelopes.

mod audio;
mod client;
mod error;
mod parse;

#[cfg(not(target_arch = "wasm32"))]
pub use audio::RawMediaResponse;
pub use audio::{AudioChunk, AudioStream};
pub use client::ApiClient;
pub use error::{ApiError, parse_error_body};
pub use halogen_wire::{LoginData, Page, StatusData, TokenData};

#[cfg(test)]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
mod transport;
#[cfg(not(target_arch = "wasm32"))]
pub use transport::{DispatchFuture, LocalTransport, MediaPathFuture, install_local_resolver};

#[cfg(not(target_arch = "wasm32"))]
mod local_media;
