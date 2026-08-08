//! On-device audio byte storage — the `MediaStore`/`MediaWriter` traits, the
//! `MediaStoreHandle`, `PartialInfo`, and `open_media_handle`. See the per-target
//! backends in `native`/`web`. `MediaStore` holds the actual audio bytes of
//! device-downloaded episodes (ground truth behind `ClientDownloadState::
//! Downloaded`); deliberately separate from `LocalStore` (small JSON metadata vs.
//! megabytes of binary with a download/remove lifecycle).

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

/// A playable device-copy source resolved by [`MediaStore::audio_url`].
///
/// Web: `url` is a fresh `blob:` object URL and `content_type` is the stored
/// blob's MIME type (captured from the download response headers at commit) —
/// the web audio backend feeds it to a `<source type=…>` child, since WebKit
/// resolves `blob:` media strictly by type. Native: `url` is an absolute file
/// path (the loopback bridge derives the content type from the extension) and
/// `content_type` is `None`.
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
    /// Open an incremental writer for an episode's audio — the streaming download
    /// path. The bytes become the episode's stored audio only on a successful
    /// [`MediaWriter::commit`]; until then they accumulate as a *partial*, which
    /// `audio_url`/`list_ids` never expose (so an interrupted download is never
    /// playable). A writer dropped before committing **keeps** its durable partial
    /// so the next attempt can resume — see [`partial`](MediaStore::partial).
    ///
    /// `resume` chooses the start: `false` truncates any existing partial and
    /// starts fresh at offset 0; `true` appends to the existing partial (the
    /// caller derives the resume offset from [`partial`](MediaStore::partial)).
    /// `total` is the full size when known (from the first chunk's `Content-Range`)
    /// — recorded so a later resume can seed a progress bar; pass `None` when
    /// unknown.
    async fn open_writer(
        &self,
        episode_id: i32,
        content_type: Option<&str>,
        total: Option<u64>,
        resume: bool,
    ) -> Result<Box<dyn MediaWriter>>;

    /// A playable source for a stored episode, or `None` when no bytes are
    /// stored. Web: a **fresh** `blob:` object URL (+ the blob's MIME type) —
    /// ownership of the URL transfers to the caller, who must `revokeObjectURL`
    /// it once displaced; the blob is probe-read (with retries) before the URL
    /// is minted, so a WebKit blob whose backing file hasn't settled yet
    /// surfaces as a retried read, not a dead URL. Native: an absolute file
    /// path. `Ok(None)` means "no committed record"; `Err` means the record
    /// exists but its bytes couldn't be read (transient platform failure).
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

/// An in-progress, append-only write of one episode's audio (from [`MediaStore::
/// open_writer`]). Chunks are streamed *durably* to backing storage so the whole
/// (often 100+ MB) file never sits in memory at once — and so a refresh/restart
/// mid-download leaves a recoverable partial rather than losing everything. The
/// accumulated bytes become the episode's *stored* (playable, listed) audio only
/// on [`commit`](MediaWriter::commit). Dropping the writer before committing
/// keeps the durable partial (invisible to `audio_url`/`list_ids`) so the next
/// attempt resumes via `open_writer(.., resume = true)`; the partial is purged
/// only by `commit`, [`remove_audio`](MediaStore::remove_audio), or
/// [`clear`](MediaStore::clear). Single use: call [`write`](MediaWriter::write)
/// zero or more times, then [`commit`](MediaWriter::commit) exactly once.
#[async_trait(?Send)]
pub trait MediaWriter {
    /// Append the next chunk of audio to backing storage, durably (it must
    /// survive a process/page restart so the download can resume).
    async fn write(&mut self, chunk: &[u8]) -> Result<()>;

    /// Atomically commit the accumulated bytes as the episode's stored audio,
    /// replacing any previous copy. Call exactly once.
    async fn commit(&mut self) -> Result<()>;
}

/// The active account's on-device audio directory (native):
/// `<data root>/{segment}/audio`. Namespaced by the active-user segment (like the
/// native metadata store's `<data root>/{segment}/halogen.db`) so accounts never
/// share downloaded bytes — a remote and the embedded server have per-server
/// episode ids that collide. Exposed so the webview loopback media server
/// (`ui-state`'s `WebviewMediaBridge`) serves from the same dir the store writes
/// to: both read the ambient segment, so they agree for the active user.
#[cfg(not(target_arch = "wasm32"))]
pub fn audio_dir() -> std::path::PathBuf {
    let segment = halogen_ui_platform::namespace::segment();
    halogen_ui_platform::paths::data_root()
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
        let segment = halogen_ui_platform::namespace::segment();
        MediaStoreHandle(Some(Rc::new(WebMediaStore::new(&segment))))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        purge_legacy_native_media();
        match NativeMediaStore::open(audio_dir()) {
            Ok(store) => MediaStoreHandle(Some(Rc::new(store))),
            Err(e) => {
                halogen_ui_logging::error!("Failed to open media store: {e}");
                MediaStoreHandle(None)
            }
        }
    }
}

/// One-time (per process) removal of the pre-namespacing global audio dir
/// (`<data root>/audio`). The per-account stores now live at
/// `<data root>/{segment}/audio`, so the old shared bytes are stale — and on a
/// device that used both a remote and the embedded server they could be a
/// *different* episode's audio (the collision this namespacing fixes). Delete
/// them once to reclaim the space; downloads re-populate per account.
/// Best-effort — a failure just leaves the old dir in place.
#[cfg(not(target_arch = "wasm32"))]
fn purge_legacy_native_media() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let legacy = halogen_ui_platform::paths::data_root().join("audio");
    if legacy.is_dir() {
        match std::fs::remove_dir_all(&legacy) {
            Ok(()) => halogen_ui_logging::info!("removed legacy device-global media dir"),
            Err(e) => halogen_ui_logging::warn!("could not remove legacy media dir: {e}"),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::audio_dir;
    use halogen_ui_platform::namespace;

    // The device audio dir is namespaced per account, so a remote and the
    // embedded server — whose per-server episode ids collide — never share (or
    // overwrite) each other's downloaded bytes. This is the C2 fix: the old
    // device-global store, keyed by bare episode id, served the wrong account's
    // audio for a colliding id.
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
