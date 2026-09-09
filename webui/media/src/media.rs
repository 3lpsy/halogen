//! On-device audio byte storage, the `MediaStore`/`MediaWriter` traits, the `MediaStoreHandle`, `PartialInfo`, and
//! `open_media_handle`. See the per-target backends in `native`/`web`. `MediaStore` holds the actual audio bytes of
//! device-downloaded episodes (ground truth behind `ClientDownloadState:: Downloaded`); deliberately separate from
//! `LocalStore` (small JSON metadata vs. megabytes of binary with a download/remove lifecycle).

#[cfg(not(target_arch = "wasm32"))]
use crate::NativeMediaStore;
#[cfg(target_arch = "wasm32")]
use crate::WebMediaStore;

use std::rc::Rc;

use anyhow::Result;
use async_trait::async_trait;

/// A cloneable handle to the one shared `MediaStore`, provided via context
/// (next to `StoreHandle`). `None` when the backend failed to open — device
/// downloads are then disabled and downloads surface as `Failed`.
#[derive(Clone)]
pub struct MediaStoreHandle(pub Option<Rc<dyn MediaStore>>);

/// Resolved device audio: web returns a fresh blob URL with MIME type for WebKit's typed source element; native returns
/// an absolute path and no type, since the media bridge derives it from the extension.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalAudio {
    pub url: String,
    pub content_type: Option<String>,
}

/// Resume info for an episode's in-progress (uncommitted) download — the durably
/// staged bytes a refresh/restart left behind. The orchestrator re-enters the
/// chunked download loop at `downloaded` (see `download_audio`).
#[derive(Clone, Debug)]
pub struct PartialInfo {
    /// Bytes durably staged so far — the byte offset to resume the download from.
    pub downloaded: u64,
    /// Expected full size (from the original `Content-Range`), if it was known.
    /// `None` means the resume must re-derive it from the next chunk's headers.
    pub total: Option<u64>,
    /// MIME type captured when the download started, so the resumed writer keeps
    /// the same content type (and, on native, the same final extension).
    pub content_type: Option<String>,
}

/// Byte storage for device-downloaded episode audio.
#[async_trait(?Send)]
pub trait MediaStore {
    /// Open a durable partial, exposed as playable only after commit. Dropping preserves it for resume; `resume=false`
    /// truncates, while true appends at the reported partial offset. `total` records the full known size for restored
    /// progress, or None when unknown.
    async fn open_writer(
        &self,
        episode_id: i32,
        content_type: Option<&str>,
        total: Option<u64>,
        resume: bool,
    ) -> Result<Box<dyn MediaWriter>>;

    /// Return committed audio, None if absent, or Err for unreadable bytes. Web probes/retries the blob before minting
    /// a fresh URL with MIME type; the caller must revoke that URL when displaced. Native returns an absolute path.
    async fn audio_url(&self, episode_id: i32) -> Result<Option<LocalAudio>>;

    /// Delete an episode's stored bytes — both a committed copy and any
    /// uncommitted partial (no-op if absent).
    async fn remove_audio(&self, episode_id: i32) -> Result<()>;

    /// Ids with bytes present — the boot-time ground truth for
    /// `ClientDownloadState::Downloaded`. Committed audio only; partials excluded.
    async fn list_ids(&self) -> Result<Vec<i32>>;

    /// Resume info for an episode's in-progress (uncommitted) download, or `None`
    /// when no partial is staged.
    async fn partial(&self, episode_id: i32) -> Result<Option<PartialInfo>>;

    /// Ids with a staged (uncommitted) partial — the boot-time set to resume.
    /// Disjoint from [`list_ids`](MediaStore::list_ids): a `commit` turns a
    /// partial into a committed copy, so an id is in at most one of the two.
    async fn list_partials(&self) -> Result<Vec<i32>>;

    /// Wipe all stored audio, committed and partial (local wipe / sign-out flows).
    async fn clear(&self) -> Result<()>;
}

/// Append chunks durably without buffering the full file. Commit once to expose playable audio; dropping keeps the
/// hidden partial for resume. Only commit, remove_audio, or clear purges it.
#[async_trait(?Send)]
pub trait MediaWriter {
    /// Append the next chunk of audio to backing storage, durably (it must
    /// survive a process/page restart so the download can resume).
    async fn write(&mut self, chunk: &[u8]) -> Result<()>;

    /// Atomically commit the accumulated bytes as the episode's stored audio,
    /// replacing any previous copy. Call exactly once.
    async fn commit(&mut self) -> Result<()>;
}

/// Return `<data root>/{segment}/audio` for the active account. Store and media bridge share this namespace so
/// colliding episode IDs across servers cannot share downloaded bytes.
#[cfg(not(target_arch = "wasm32"))]
pub fn audio_dir() -> std::path::PathBuf {
    let segment = halogen_webui_platform::namespace::segment();
    halogen_webui_platform::paths::data_root()
        .join(segment)
        .join("audio")
}

/// Open the active account's platform media store, wrapped for context. The
/// segment comes from the ambient active-user namespace (set by `AccountsProvider`
/// before the keyed data subtree mounts), so a user switch's remount re-opens the
/// new account's store — exactly like `open_store_handle`.
pub fn open_media_handle() -> MediaStoreHandle {
    #[cfg(target_arch = "wasm32")]
    {
        let segment = halogen_webui_platform::namespace::segment();
        MediaStoreHandle(Some(Rc::new(WebMediaStore::new(&segment))))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        purge_legacy_native_media();
        match NativeMediaStore::open(audio_dir()) {
            Ok(store) => MediaStoreHandle(Some(Rc::new(store))),
            Err(e) => {
                halogen_webui_logging::error!("Failed to open media store: {e}");
                MediaStoreHandle(None)
            }
        }
    }
}

/// Best-effort, once-per-process removal of the legacy global audio directory. Its unnamespaced episode IDs may refer
/// to another server's audio; downloads repopulate account-specific directories.
#[cfg(not(target_arch = "wasm32"))]
fn purge_legacy_native_media() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let legacy = halogen_webui_platform::paths::data_root().join("audio");
    if legacy.is_dir() {
        match std::fs::remove_dir_all(&legacy) {
            Ok(()) => halogen_webui_logging::info!("removed legacy device-global media dir"),
            Err(e) => halogen_webui_logging::warn!("could not remove legacy media dir: {e}"),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::audio_dir;
    use halogen_webui_platform::namespace;

    // The device audio dir is namespaced per account, so a remote and the embedded server, whose per-server episode ids
    // collide, never share (or overwrite) each other's downloaded bytes. This is the C2 fix: the old device-global
    // store, keyed by bare episode id, served the wrong account's audio for a colliding id.
    #[test]
    fn audio_dir_is_namespaced_by_account_segment() {
        namespace::set_active(Some(7), false, 0xab);
        let remote = audio_dir();
        namespace::set_active(Some(7), false, 0xcd);
        let remote_other_server = audio_dir();
        namespace::set_active(Some(7), true, 0);
        let embedded = audio_dir();
        namespace::set_active(None, false, 0);
        let anon = audio_dir();
        namespace::set_active(None, false, 0); // leave the global sentinel clean

        assert!(
            remote.to_string_lossy().contains("u7-"),
            "remote dir: {remote:?}"
        );
        assert!(embedded.ends_with("e7/audio"), "embedded dir: {embedded:?}");
        assert!(anon.ends_with("anon/audio"), "anon dir: {anon:?}");
        assert_ne!(
            remote, embedded,
            "same id on a remote vs the embedded server must not share an audio dir"
        );
        assert_ne!(
            remote, remote_other_server,
            "same id on DIFFERENT remote servers must not share an audio dir (H5)"
        );
    }
}
