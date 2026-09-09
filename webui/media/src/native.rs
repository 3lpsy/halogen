//! Store each episode under the account's audio directory with a MIME-derived extension. Durable partial files and
//! metadata support resumed appends; commit renames atomically and partials stay unplayable. Downloads use bearer auth;
//! playback maps absolute paths through the Range-capable loopback bridge.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{LocalAudio, MediaStore, MediaWriter, PartialInfo};

pub struct NativeMediaStore {
    dir: PathBuf,
}

impl NativeMediaStore {
    pub fn open(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, episode_id: i32, ext: &str) -> PathBuf {
        self.dir.join(format!("{episode_id}.{ext}"))
    }

    fn partial_path(&self, episode_id: i32) -> PathBuf {
        self.dir.join(format!("{episode_id}{PARTIAL_SUFFIX}"))
    }

    fn partial_meta_path(&self, episode_id: i32) -> PathBuf {
        self.dir.join(format!("{episode_id}{PARTIAL_META_SUFFIX}"))
    }

    /// Any *committed* stored file for this id, regardless of extension. Skips the
    /// in-progress `<id>.partial` data file and its `<id>.partial.meta` sidecar
    /// (neither is playable yet).
    fn find(&self, episode_id: i32) -> Option<PathBuf> {
        let prefix = format!("{episode_id}.");
        std::fs::read_dir(&self.dir).ok()?.find_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?;
            (name.starts_with(&prefix) && !is_partial_artifact(name)).then_some(path)
        })
    }

    /// Walk the audio dir, mapping each entry's path to an id via `pick` (paths it
    /// returns `None` for are skipped) — the shared `read_dir` collect behind
    /// `list_ids` (committed audio) and `list_partials` (in-progress downloads).
    fn ids_from_dir(&self, pick: impl Fn(&Path) -> Option<i32>) -> Result<Vec<i32>> {
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            if let Some(id) = pick(&entry?.path()) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
}

/// Extension marking an in-progress download (a temp file `commit` renames onto
/// the real path). Excluded from `find`/`list_ids` so a partial never looks done.
const PARTIAL_SUFFIX: &str = ".partial";
/// Sidecar holding the download's content type + total size, so an interrupted
/// download can resume with the same type and seed a progress bar.
const PARTIAL_META_SUFFIX: &str = ".partial.meta";

/// True for a `<id>.partial` data file or its `<id>.partial.meta` sidecar —
/// neither is committed audio, so readers (`find`/`list_ids`) skip both.
fn is_partial_artifact(name: &str) -> bool {
    name.ends_with(PARTIAL_SUFFIX) || name.ends_with(PARTIAL_META_SUFFIX)
}

/// Persisted alongside a `<id>.partial` file. The `downloaded` offset is *not*
/// stored here — it's the live `.partial` file length, which can't drift from the
/// bytes actually on disk.
#[derive(Serialize, Deserialize)]
struct PartialMeta {
    content_type: Option<String>,
    total: Option<u64>,
}

/// Remove every `<id>.*` file in `dir` except `keep` (the just-committed final
/// file), so a re-download leaves no orphan under a previous extension and the
/// `.partial`/`.partial.meta` artifacts are cleaned up.
fn remove_existing(dir: &Path, episode_id: i32, keep: &Path) -> Result<()> {
    let prefix = format!("{episode_id}.");
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path != keep
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn ext_for(content_type: Option<&str>) -> &'static str {
    match content_type.map(|c| c.split(';').next().unwrap_or(c).trim()) {
        Some("audio/mpeg") | Some("audio/mp3") => "mp3",
        Some("audio/mp4") | Some("audio/x-m4a") | Some("audio/aac") => "m4a",
        Some("audio/ogg") => "ogg",
        Some("audio/opus") => "opus",
        Some("audio/wav") | Some("audio/x-wav") => "wav",
        Some("audio/flac") => "flac",
        _ => "bin",
    }
}

#[async_trait(?Send)]
impl MediaStore for NativeMediaStore {
    async fn open_writer(
        &self,
        episode_id: i32,
        content_type: Option<&str>,
        total: Option<u64>,
        resume: bool,
    ) -> Result<Box<dyn MediaWriter>> {
        let final_path = self.path_for(episode_id, ext_for(content_type));
        let tmp_path = self.partial_path(episode_id);
        let meta_path = self.partial_meta_path(episode_id);
        // resume → append to the existing partial; fresh → truncate/create. Either
        // way creating the file up front surfaces a quota/permission error before
        // the download starts.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resume)
            .truncate(!resume)
            .open(&tmp_path)?;
        // Record (or refresh, idempotently on resume) the type + total so a later
        // resume keeps the extension and can seed progress.
        let meta = PartialMeta {
            content_type: content_type.map(str::to_string),
            total,
        };
        std::fs::write(&meta_path, serde_json::to_vec(&meta)?)?;
        Ok(Box::new(NativeMediaWriter {
            dir: self.dir.clone(),
            episode_id,
            final_path,
            tmp_path,
            file: Some(file),
        }))
    }

    async fn audio_url(&self, episode_id: i32) -> Result<Option<LocalAudio>> {
        // Content type is `None`: the webview loopback bridge that serves this
        // path derives it from the file extension (which commit named from the
        // download's MIME type), so nothing here needs to carry it.
        Ok(self.find(episode_id).map(|p| LocalAudio {
            url: p.display().to_string(),
            content_type: None,
        }))
    }

    async fn remove_audio(&self, episode_id: i32) -> Result<()> {
        halogen_webui_logging::debug!(episode_id, "Removing device audio");
        if let Some(path) = self.find(episode_id) {
            std::fs::remove_file(path)?;
        }
        // Also drop any in-progress partial + its sidecar.
        for path in [
            self.partial_path(episode_id),
            self.partial_meta_path(episode_id),
        ] {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    async fn list_ids(&self) -> Result<Vec<i32>> {
        self.ids_from_dir(|path| {
            let name = path.file_name().and_then(|n| n.to_str())?;
            // Skip in-progress partials — only committed audio counts.
            if is_partial_artifact(name) {
                return None;
            }
            path.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|stem| stem.parse::<i32>().ok())
        })
    }

    async fn partial(&self, episode_id: i32) -> Result<Option<PartialInfo>> {
        let Ok(md) = std::fs::metadata(self.partial_path(episode_id)) else {
            return Ok(None);
        };
        // The `.partial` file length IS the resume offset — always consistent with
        // the bytes on disk. The sidecar adds the type + total (best-effort).
        let (content_type, total) = match std::fs::read(self.partial_meta_path(episode_id)) {
            Ok(bytes) => serde_json::from_slice::<PartialMeta>(&bytes)
                .map(|m| (m.content_type, m.total))
                .unwrap_or((None, None)),
            Err(_) => (None, None),
        };
        Ok(Some(PartialInfo {
            downloaded: md.len(),
            total,
            content_type,
        }))
    }

    async fn list_partials(&self) -> Result<Vec<i32>> {
        self.ids_from_dir(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(PARTIAL_SUFFIX))
                .and_then(|stem| stem.parse::<i32>().ok())
        })
    }

    async fn clear(&self) -> Result<()> {
        halogen_webui_logging::debug!("Clearing all device audio");
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.is_file() {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

/// Streams chunks to a `<id>.partial` temp file, then atomically `rename`s it onto the real path on `commit`. A writer
/// dropped before committing *keeps* the temp file (and its sidecar), so an interrupted download resumes from the bytes
/// on disk; readers (`find`/`list_ids`) still never see a partial. The partial is purged only by `commit`,
/// `remove_audio`, or `clear`.
struct NativeMediaWriter {
    dir: PathBuf,
    episode_id: i32,
    final_path: PathBuf,
    tmp_path: PathBuf,
    /// `Some` until `commit` takes it; `None` means "already committed".
    file: Option<std::fs::File>,
}

#[async_trait(?Send)]
impl MediaWriter for NativeMediaWriter {
    async fn write(&mut self, chunk: &[u8]) -> Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("media writer used after commit"))?;
        file.write_all(chunk)?;
        // Push to the OS so the bytes survive a process/page restart (resume reads
        // the file length). Cheap — not an fsync, just out of our buffer.
        file.flush()?;
        Ok(())
    }

    async fn commit(&mut self) -> Result<()> {
        let mut file = self
            .file
            .take()
            .ok_or_else(|| anyhow::anyhow!("media writer already committed"))?;
        file.flush()?;
        // fsync BEFORE the rename: a crash/power loss can otherwise persist the rename (directory metadata) while the
        // data pages never hit disk, a truncated file permanently marked Downloaded. With the sync, the bad orderings
        // collapse to "rename lost", which leaves the durable partial to resume. One fsync per completed download,
        // negligible.
        file.sync_all()?;
        drop(file); // close before rename (some platforms won't rename an open file)
        // Promote the temp file atomically first (so a crash here still leaves a
        // complete, committed file), then sweep the meta sidecar and any previous
        // copy under a different extension.
        std::fs::rename(&self.tmp_path, &self.final_path)?;
        remove_existing(&self.dir, self.episode_id, &self.final_path)?;
        halogen_webui_logging::debug!(episode_id = self.episode_id, "Committed device audio");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> NativeMediaStore {
        let dir =
            std::env::temp_dir().join(format!("halogen-media-test-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        NativeMediaStore::open(dir).unwrap()
    }

    /// Write a whole buffer in one shot via the streaming API — the path the
    /// product actually uses (`open_writer` → `write` → `commit`). Keeps the
    /// roundtrip tests terse.
    async fn store_audio(
        s: &NativeMediaStore,
        id: i32,
        bytes: &[u8],
        ct: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut w = s
            .open_writer(id, ct, Some(bytes.len() as u64), false)
            .await?;
        w.write(bytes).await?;
        w.commit().await
    }

    #[tokio::test]
    async fn roundtrip_store_list_url_remove() {
        let s = store("roundtrip");
        assert!(s.list_ids().await.unwrap().is_empty());
        assert!(s.audio_url(7).await.unwrap().is_none());

        store_audio(&s, 7, b"abc", Some("audio/mpeg"))
            .await
            .unwrap();
        store_audio(&s, 9, b"def", None).await.unwrap();

        let mut ids = s.list_ids().await.unwrap();
        ids.sort();
        assert_eq!(ids, vec![7, 9]);

        let url = s.audio_url(7).await.unwrap().expect("stored").url;
        assert!(url.ends_with("7.mp3"), "mime-derived extension: {url}");
        assert_eq!(std::fs::read(&url).unwrap(), b"abc");

        // Re-store with a different type replaces the old file (no orphans).
        store_audio(&s, 7, b"xyz", Some("audio/ogg")).await.unwrap();
        let url = s.audio_url(7).await.unwrap().expect("stored").url;
        assert!(url.ends_with("7.ogg"));
        assert_eq!(s.list_ids().await.unwrap().len(), 2);

        s.remove_audio(7).await.unwrap();
        assert!(s.audio_url(7).await.unwrap().is_none());

        s.clear().await.unwrap();
        assert!(s.list_ids().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn streaming_writer_assembles_chunks_on_commit() {
        let s = store("streaming");
        let mut w = s
            .open_writer(5, Some("audio/mpeg"), Some(11), false)
            .await
            .unwrap();
        w.write(b"hel").await.unwrap();
        w.write(b"lo ").await.unwrap();
        w.write(b"world").await.unwrap();
        // Uncommitted: the temp file exists but counts as nothing downloaded.
        assert!(s.audio_url(5).await.unwrap().is_none());
        assert!(s.list_ids().await.unwrap().is_empty());

        w.commit().await.unwrap();
        let url = s.audio_url(5).await.unwrap().expect("committed").url;
        assert!(url.ends_with("5.mp3"));
        assert_eq!(std::fs::read(&url).unwrap(), b"hello world");
        assert_eq!(s.list_ids().await.unwrap(), vec![5]);
        // Commit cleaned up the partial + sidecar.
        assert!(s.partial(5).await.unwrap().is_none());
        assert!(s.list_partials().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn dropped_writer_keeps_resumable_partial() {
        let s = store("dropped");
        {
            let mut w = s
                .open_writer(8, Some("audio/mpeg"), Some(14), false)
                .await
                .unwrap();
            w.write(b"partial").await.unwrap();
            // dropped without commit
        }
        // Not committed: not playable, not listed as downloaded…
        assert!(s.audio_url(8).await.unwrap().is_none(), "nothing committed");
        assert!(s.list_ids().await.unwrap().is_empty(), "partial not listed");
        // …but the partial survives for resume, with its offset + meta.
        let info = s.partial(8).await.unwrap().expect("partial kept");
        assert_eq!(info.downloaded, 7);
        assert_eq!(info.total, Some(14));
        assert_eq!(info.content_type.as_deref(), Some("audio/mpeg"));
        assert_eq!(s.list_partials().await.unwrap(), vec![8]);
    }

    #[tokio::test]
    async fn resume_appends_then_commits_full_file() {
        let s = store("resume");
        // First attempt stores half, then is interrupted (writer dropped).
        {
            let mut w = s
                .open_writer(4, Some("audio/mpeg"), Some(11), false)
                .await
                .unwrap();
            w.write(b"hello ").await.unwrap();
        }
        let info = s.partial(4).await.unwrap().expect("partial");
        assert_eq!(info.downloaded, 6);

        // Resume: append the rest from the stored offset, then commit.
        let mut w = s
            .open_writer(4, info.content_type.as_deref(), info.total, true)
            .await
            .unwrap();
        w.write(b"world").await.unwrap();
        w.commit().await.unwrap();

        let url = s.audio_url(4).await.unwrap().expect("committed").url;
        assert_eq!(std::fs::read(&url).unwrap(), b"hello world");
        assert_eq!(s.list_ids().await.unwrap(), vec![4]);
        assert!(s.partial(4).await.unwrap().is_none(), "partial consumed");
    }

    #[tokio::test]
    async fn remove_audio_clears_partial() {
        let s = store("remove-partial");
        {
            let mut w = s.open_writer(3, None, Some(9), false).await.unwrap();
            w.write(b"partial").await.unwrap();
        }
        assert!(s.partial(3).await.unwrap().is_some());
        s.remove_audio(3).await.unwrap();
        assert!(s.partial(3).await.unwrap().is_none(), "partial removed");
        assert!(s.list_partials().await.unwrap().is_empty());
        assert!(
            !s.dir.join("3.partial").exists() && !s.dir.join("3.partial.meta").exists(),
            "partial + sidecar gone"
        );
    }

    #[tokio::test]
    async fn re_download_replaces_old_extension() {
        let s = store("replace");
        store_audio(&s, 3, b"old", Some("audio/mpeg"))
            .await
            .unwrap();
        store_audio(&s, 3, b"new", Some("audio/ogg")).await.unwrap();
        let url = s.audio_url(3).await.unwrap().expect("stored").url;
        assert!(url.ends_with("3.ogg"));
        assert_eq!(std::fs::read(&url).unwrap(), b"new");
        assert_eq!(s.list_ids().await.unwrap(), vec![3], "no orphan mp3");
    }
}
