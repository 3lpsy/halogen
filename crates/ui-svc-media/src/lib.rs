//! On-device audio byte storage. Re-export shell; the `MediaStore`/`MediaWriter`
//! traits + `MediaStoreHandle`/`PartialInfo`/`open_media_handle` live in [`media`],
//! with per-target backends in `native` (files) / `web` (IndexedDB blobs).

mod media;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use media::audio_dir;
pub use media::{
    LocalAudio, MediaStore, MediaStoreHandle, MediaWriter, PartialInfo, open_media_handle,
};
#[cfg(not(target_arch = "wasm32"))]
pub use native::NativeMediaStore;
#[cfg(target_arch = "wasm32")]
pub use web::{WebMediaStore, revoke_object_url};
