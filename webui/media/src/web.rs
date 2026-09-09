//! IndexedDB holds committed audio blobs, durable partial chunks, and resume metadata; combining blobs avoids full-file
//! wasm heap copies. Missing stores trigger a versioned repair. WebKit can expose records before backing files settle,
//! so audio_url probes bytes and retries fresh reads before returning a URL.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use idb::{Database, DatabaseEvent, Factory, ObjectStoreParams, TransactionMode};
use js_sys::Reflect;
use wasm_bindgen::JsValue;

use super::{LocalAudio, MediaStore, MediaWriter, PartialInfo};
use halogen_webui_platform::store_keys::{
    MEDIA_AUDIO_STORE as STORE, MEDIA_DB_PREFIX, MEDIA_META_STORE as STORE_META,
    MEDIA_PARTIAL_STORE as STORE_PARTIAL, MEDIA_STORES as STORES,
};
use halogen_webui_store_idb::{commit_tx, idb_err, one_store_tx};

pub struct WebMediaStore {
    /// Per-account database name: `halogen.media.{segment}`. Namespaced like the
    /// metadata/config stores so accounts (a remote and the embedded server can
    /// have colliding episode ids) never share downloaded audio.
    db_name: String,
    /// Lazily-opened connection (the provider constructs stores synchronously;
    /// IndexedDB opening is async). `Rc` because `idb::Database` isn't `Clone`.
    /// A racing double-open is harmless — both resolve to the same database.
    db: RefCell<Option<Rc<Database>>>,
}

impl WebMediaStore {
    /// Open the media store for the namespace `segment` (`u{id}` / `e{id}` /
    /// `anon`), mirroring `WebLocalStore::new`.
    pub fn new(segment: &str) -> Self {
        Self {
            db_name: format!("{MEDIA_DB_PREFIX}{segment}"),
            db: RefCell::new(None),
        }
    }

    async fn db(&self) -> Result<Rc<Database>> {
        if let Some(db) = self.db.borrow().as_ref() {
            return Ok(db.clone());
        }
        let db = Rc::new(open_with_store(&self.db_name).await?);
        *self.db.borrow_mut() = Some(db.clone());
        Ok(db)
    }
}

/// Ensure every store exists after baseline open. If an interrupted or legacy upgrade left one missing, reopen at the
/// next version to force its creation.
async fn open_with_store(db_name: &str) -> Result<Database> {
    let factory = Factory::new().map_err(idb_err("indexeddb factory"))?;

    // Open at the DB's *current* version (`None`) — never a fixed number, so a
    // launch after a repair bumped the version can't trip a downgrade error. A
    // fresh DB still fires `on_upgrade_needed` (0 → 1) and gets the stores; an
    // existing DB that already has them is returned as-is.
    let db = open_at(&factory, db_name, None).await?;
    if STORES
        .iter()
        .all(|s| db.store_names().iter().any(|n| n == s))
    {
        return Ok(db);
    }

    // Legacy/broken DB sitting at its version *without* some store: force an
    // upgrade by reopening one version higher, which re-runs store creation.
    let next = db
        .version()
        .map_err(idb_err("indexeddb version"))?
        .saturating_add(1);
    halogen_webui_logging::warn!(next_version = next, "media DB missing a store; repairing");
    db.close();
    open_at(&factory, db_name, Some(next)).await
}

/// Open the DB at `version` (`None` = current/default), creating any missing
/// store in [`STORES`] on upgrade.
async fn open_at(factory: &Factory, db_name: &str, version: Option<u32>) -> Result<Database> {
    let mut open = factory
        .open(db_name, version)
        .map_err(idb_err("indexeddb open"))?;
    open.on_upgrade_needed(|event| {
        if let Ok(db) = event.database() {
            for name in STORES {
                if !db.store_names().iter().any(|n| n == name)
                    && let Err(e) = db.create_object_store(name, ObjectStoreParams::new())
                {
                    halogen_webui_logging::error!("Failed to create media '{name}' store: {e}");
                }
            }
        }
    });
    open.await.map_err(idb_err("indexeddb open"))
}

fn key(episode_id: i32) -> JsValue {
    JsValue::from_f64(episode_id as f64)
}

/// Collect an object store's keys as episode ids (keys are stored as JS numbers).
/// Shared by `list_ids` (committed `audio`) and `list_partials` (`audio_partial_meta`).
async fn get_all_ids(db: &Database, store: &str) -> Result<Vec<i32>> {
    let (tx, s) = one_store_tx(db, store, TransactionMode::ReadOnly)?;
    let keys = s
        .get_all_keys(None, None)
        .map_err(idb_err("keys"))?
        .await
        .map_err(idb_err("keys"))?;
    tx.await.map_err(idb_err("tx done"))?;
    Ok(keys
        .into_iter()
        .filter_map(|k| k.as_f64())
        .map(|f| f as i32)
        .collect())
}

/// Extra `read_and_probe_audio` attempts after the first (delays 250ms, 500ms,
/// 1s, 2s — ~3.75s total). Sized to cover WebKit's blob-file settle window
/// without stretching the Loading spinner unreasonably; the player adds its own
/// outer retry on top (see `ui-svc-player`'s local-error retry).
const AUDIO_PROBE_RETRIES: u32 = 4;

/// Probe nonempty committed blobs by reading their first and last bytes before minting URLs. WebKit backing-file races
/// become retryable errors here instead of opaque media failures. Return MIME type when present, None for an absent
/// record.
async fn read_and_probe_audio(
    db: &Database,
    episode_id: i32,
) -> Result<Option<(web_sys::Blob, Option<String>)>> {
    let (tx, store) = one_store_tx(db, STORE, TransactionMode::ReadOnly)?;
    let value: Option<JsValue> = store
        .get(key(episode_id))
        .map_err(idb_err("get"))?
        .await
        .map_err(idb_err("get"))?;
    tx.await.map_err(idb_err("tx done"))?;
    let Some(value) = value else { return Ok(None) };
    let blob: web_sys::Blob = value.into();
    let size = blob.size();
    if size <= 0.0 {
        return Err(anyhow!("committed audio blob reads as empty (size 0)"));
    }
    probe_byte(&blob, 0.0).await?;
    probe_byte(&blob, size - 1.0).await?;
    let content_type = Some(blob.type_()).filter(|t| !t.is_empty());
    Ok(Some((blob, content_type)))
}

/// Read one byte at `offset` out of `blob`, surfacing a read failure as `Err`.
async fn probe_byte(blob: &web_sys::Blob, offset: f64) -> Result<()> {
    let slice = blob
        .slice_with_f64_and_f64(offset, offset + 1.0)
        .map_err(|e| anyhow!("blob slice at {offset}: {e:?}"))?;
    wasm_bindgen_futures::JsFuture::from(slice.array_buffer())
        .await
        .map_err(|e| anyhow!("blob read at {offset}: {e:?}"))?;
    Ok(())
}

/// Revoke a `blob:` object URL minted by [`audio_url`](WebMediaStore::audio_url).
/// The audio backend revokes once a URL is displaced; this is the escape hatch
/// for any caller that resolved a fresh `Some(url)` but never handed it to the
/// element (e.g. a superseded load), so the blob isn't pinned for the page life.
pub fn revoke_object_url(url: &str) {
    web_sys::Url::revoke_object_url(url).ok();
}

/// Key for one in-progress chunk: `"{id}.{seq}"`. String keys keep an episode's
/// chunks grouped and unique across episodes.
fn chunk_key(episode_id: i32, seq: u32) -> JsValue {
    JsValue::from_str(&format!("{episode_id}.{seq}"))
}

/// Resume bookkeeping for an in-progress download (the `audio_partial_meta`
/// record). `next_seq` is also the chunk count (keys `0..next_seq`).
struct PartialMetaRec {
    content_type: String,
    total: Option<u64>,
    downloaded: u64,
    next_seq: u32,
}

fn meta_to_js(rec: &PartialMetaRec) -> JsValue {
    let o = js_sys::Object::new();
    let _ = Reflect::set(
        &o,
        &"content_type".into(),
        &rec.content_type.as_str().into(),
    );
    let _ = Reflect::set(
        &o,
        &"total".into(),
        &rec.total
            .map_or(JsValue::NULL, |t| JsValue::from_f64(t as f64)),
    );
    let _ = Reflect::set(
        &o,
        &"downloaded".into(),
        &JsValue::from_f64(rec.downloaded as f64),
    );
    let _ = Reflect::set(
        &o,
        &"next_seq".into(),
        &JsValue::from_f64(rec.next_seq as f64),
    );
    o.into()
}

fn meta_from_js(v: &JsValue) -> PartialMetaRec {
    let get = |k: &str| Reflect::get(v, &k.into()).ok();
    PartialMetaRec {
        content_type: get("content_type")
            .and_then(|j| j.as_string())
            .unwrap_or_else(|| "audio/mpeg".to_string()),
        total: get("total").and_then(|j| j.as_f64()).map(|f| f as u64),
        downloaded: get("downloaded").and_then(|j| j.as_f64()).unwrap_or(0.0) as u64,
        next_seq: get("next_seq").and_then(|j| j.as_f64()).unwrap_or(0.0) as u32,
    }
}

async fn read_partial_meta(db: &Database, episode_id: i32) -> Result<Option<PartialMetaRec>> {
    let (tx, store) = one_store_tx(db, STORE_META, TransactionMode::ReadOnly)?;
    let value: Option<JsValue> = store
        .get(key(episode_id))
        .map_err(idb_err("get"))?
        .await
        .map_err(idb_err("get"))?;
    tx.await.map_err(idb_err("tx done"))?;
    Ok(value.map(|v| meta_from_js(&v)))
}

/// Delete an episode's in-progress chunks and its meta record in one transaction. Chunks are found by sweeping the
/// `audio_partial` store for keys prefixed `"{id}."` rather than counting `0..next_seq` from the meta, so it also
/// reclaims orphan chunk blobs whose meta record vanished (a partial wipe that missed the meta), which `next_seq` alone
/// (0 with no meta) could never reach. No-op when the episode has neither chunks nor a meta record.
async fn discard_partial(db: &Database, episode_id: i32) -> Result<()> {
    let tx = db
        .transaction(&[STORE_PARTIAL, STORE_META], TransactionMode::ReadWrite)
        .map_err(idb_err("tx"))?;
    let parts = tx.object_store(STORE_PARTIAL).map_err(idb_err("store"))?;
    let prefix = format!("{episode_id}.");
    let keys = parts
        .get_all_keys(None, None)
        .map_err(idb_err("keys"))?
        .await
        .map_err(idb_err("keys"))?;
    for k in keys {
        if k.as_string().is_some_and(|s| s.starts_with(&prefix)) {
            parts
                .delete(k)
                .map_err(idb_err("delete chunk"))?
                .await
                .map_err(idb_err("delete chunk"))?;
        }
    }
    let meta_store = tx.object_store(STORE_META).map_err(idb_err("store"))?;
    meta_store
        .delete(key(episode_id))
        .map_err(idb_err("delete meta"))?
        .await
        .map_err(idb_err("delete meta"))?;
    commit_tx(tx).await?;
    Ok(())
}

#[async_trait(?Send)]
impl MediaStore for WebMediaStore {
    async fn open_writer(
        &self,
        episode_id: i32,
        content_type: Option<&str>,
        total: Option<u64>,
        resume: bool,
    ) -> Result<Box<dyn MediaWriter>> {
        let db = self.db().await?;
        let rec = if resume {
            // Continue the staged partial; fall back to the caller's hints if the
            // meta record somehow vanished (treated as a fresh start at 0).
            match read_partial_meta(&db, episode_id).await? {
                Some(mut m) => {
                    if let Some(ct) = content_type {
                        m.content_type = ct.to_string();
                    }
                    m.total = m.total.or(total);
                    m
                }
                None => {
                    // Meta vanished but chunk blobs may have survived (a partial
                    // wipe that missed the meta). Starting fresh at `next_seq: 0`
                    // without clearing them would let orphan chunks `{id}.0..N` mix
                    // into a later `commit` and corrupt it — discard them first.
                    discard_partial(&db, episode_id).await?;
                    PartialMetaRec {
                        content_type: content_type.unwrap_or("audio/mpeg").to_string(),
                        total,
                        downloaded: 0,
                        next_seq: 0,
                    }
                }
            }
        } else {
            // Fresh start: drop any stale partial, then seed a zero record so the
            // download surfaces via `partial`/`list_partials` from the first byte.
            discard_partial(&db, episode_id).await?;
            let rec = PartialMetaRec {
                content_type: content_type.unwrap_or("audio/mpeg").to_string(),
                total,
                downloaded: 0,
                next_seq: 0,
            };
            put_meta(&db, episode_id, &rec).await?;
            rec
        };
        Ok(Box::new(WebMediaWriter {
            db,
            episode_id,
            content_type: rec.content_type,
            total: rec.total,
            downloaded: rec.downloaded,
            next_seq: rec.next_seq,
        }))
    }

    async fn audio_url(&self, episode_id: i32) -> Result<Option<LocalAudio>> {
        let db = self.db().await?;
        // Retry fresh IndexedDB gets until the blob bytes can be probed. Reusing the same handle can retain WebKit's
        // transient record/backing-file race and yield an unplayable URL.
        let mut delay_ms: u32 = 250;
        for attempt in 0..=AUDIO_PROBE_RETRIES {
            match read_and_probe_audio(&db, episode_id).await {
                Ok(None) => return Ok(None),
                Ok(Some((blob, content_type))) => {
                    if attempt > 0 {
                        halogen_webui_logging::info!(
                            episode_id,
                            attempt,
                            "device audio read recovered after retry"
                        );
                    }
                    let url = web_sys::Url::create_object_url_with_blob(&blob)
                        .map_err(|e| anyhow!("object url: {e:?}"))?;
                    return Ok(Some(LocalAudio { url, content_type }));
                }
                Err(e) if attempt < AUDIO_PROBE_RETRIES => {
                    halogen_webui_logging::warn!(
                        episode_id,
                        attempt,
                        error = %e,
                        "device audio blob failed its probe read; retrying"
                    );
                    halogen_webui_platform::time::sleep_ms(delay_ms).await;
                    delay_ms = delay_ms.saturating_mul(2);
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("probe loop always returns")
    }

    async fn remove_audio(&self, episode_id: i32) -> Result<()> {
        let db = self.db().await?;
        halogen_webui_logging::debug!(episode_id, "Removing device audio");
        let (tx, store) = one_store_tx(&db, STORE, TransactionMode::ReadWrite)?;
        store
            .delete(key(episode_id))
            .map_err(idb_err("delete"))?
            .await
            .map_err(idb_err("delete"))?;
        commit_tx(tx).await?;
        // Also drop any in-progress partial for this id.
        discard_partial(&db, episode_id).await?;
        Ok(())
    }

    async fn list_ids(&self) -> Result<Vec<i32>> {
        let db = self.db().await?;
        get_all_ids(&db, STORE).await
    }

    async fn partial(&self, episode_id: i32) -> Result<Option<PartialInfo>> {
        let db = self.db().await?;
        Ok(read_partial_meta(&db, episode_id)
            .await?
            .map(|m| PartialInfo {
                downloaded: m.downloaded,
                total: m.total,
                content_type: Some(m.content_type),
            }))
    }

    async fn list_partials(&self) -> Result<Vec<i32>> {
        let db = self.db().await?;
        get_all_ids(&db, STORE_META).await
    }

    async fn clear(&self) -> Result<()> {
        let db = self.db().await?;
        halogen_webui_logging::debug!("Clearing all device audio");
        let tx = db
            .transaction(&STORES, TransactionMode::ReadWrite)
            .map_err(idb_err("tx"))?;
        for name in STORES {
            let store = tx.object_store(name).map_err(idb_err("store"))?;
            store
                .clear()
                .map_err(idb_err("clear"))?
                .await
                .map_err(idb_err("clear"))?;
        }
        commit_tx(tx).await?;
        Ok(())
    }
}

/// Write (replace) an episode's `audio_partial_meta` record.
async fn put_meta(db: &Database, episode_id: i32, rec: &PartialMetaRec) -> Result<()> {
    let (tx, store) = one_store_tx(db, STORE_META, TransactionMode::ReadWrite)?;
    store
        .put(&meta_to_js(rec), Some(&key(episode_id)))
        .map_err(idb_err("put meta"))?
        .await
        .map_err(idb_err("put meta"))?;
    commit_tx(tx).await?;
    Ok(())
}

fn blob_from_bytes(chunk: &[u8]) -> Result<web_sys::Blob> {
    let array = js_sys::Uint8Array::from(chunk);
    let seq = js_sys::Array::of1(&array);
    web_sys::Blob::new_with_u8_array_sequence(&seq).map_err(|e| anyhow!("chunk blob: {e:?}"))
}

/// Persist each chunk and resume metadata, then combine typed blobs and clear partials on commit. Only one chunk
/// occupies the wasm heap at a time; dropping leaves the partial available for resume.
struct WebMediaWriter {
    db: Rc<Database>,
    episode_id: i32,
    content_type: String,
    total: Option<u64>,
    downloaded: u64,
    next_seq: u32,
}

#[async_trait(?Send)]
impl MediaWriter for WebMediaWriter {
    async fn write(&mut self, chunk: &[u8]) -> Result<()> {
        let blob = blob_from_bytes(chunk)?;
        let next_downloaded = self.downloaded + chunk.len() as u64;
        let next_seq = self.next_seq + 1;
        // Persist the chunk and the advanced bookkeeping in ONE transaction, so a
        // refresh never sees a chunk without its meta (or vice-versa).
        let tx = self
            .db
            .transaction(&[STORE_PARTIAL, STORE_META], TransactionMode::ReadWrite)
            .map_err(idb_err("tx"))?;
        let parts = tx.object_store(STORE_PARTIAL).map_err(idb_err("store"))?;
        let value: JsValue = blob.into();
        parts
            .put(&value, Some(&chunk_key(self.episode_id, self.next_seq)))
            .map_err(idb_err("put chunk"))?
            .await
            .map_err(idb_err("put chunk (quota?)"))?;
        let meta_store = tx.object_store(STORE_META).map_err(idb_err("store"))?;
        let rec = PartialMetaRec {
            content_type: self.content_type.clone(),
            total: self.total,
            downloaded: next_downloaded,
            next_seq,
        };
        meta_store
            .put(&meta_to_js(&rec), Some(&key(self.episode_id)))
            .map_err(idb_err("put meta"))?
            .await
            .map_err(idb_err("put meta"))?;
        commit_tx(tx).await?;
        self.downloaded = next_downloaded;
        self.next_seq = next_seq;
        Ok(())
    }

    async fn commit(&mut self) -> Result<()> {
        halogen_webui_logging::debug!(episode_id = self.episode_id, "Committing device audio");
        // Gather the staged chunk blobs in order (blobs are disk-backed — combining
        // them doesn't pull the bytes through the wasm heap).
        let parts = js_sys::Array::new();
        let mut missing: Option<u32> = None;
        {
            let tx = self
                .db
                .transaction(&[STORE_PARTIAL], TransactionMode::ReadOnly)
                .map_err(idb_err("tx"))?;
            let store = tx.object_store(STORE_PARTIAL).map_err(idb_err("store"))?;
            for seq in 0..self.next_seq {
                let value: Option<JsValue> = store
                    .get(chunk_key(self.episode_id, seq))
                    .map_err(idb_err("get chunk"))?
                    .await
                    .map_err(idb_err("get chunk"))?;
                match value {
                    Some(v) => {
                        parts.push(&v);
                    }
                    None => {
                        missing = Some(seq);
                        break;
                    }
                }
            }
            tx.await.map_err(idb_err("tx done"))?;
        }
        // A chunk vanished BELOW `next_seq` (IndexedDB corruption, or a partial-store wipe that missed the meta).
        // Resume re-fetches only from `next_seq`, so it can never backfill a gap below it, committing here would fail
        // forever. Discard the partial instead, so the next attempt restarts cleanly from byte 0 rather than wedging on
        // a permanently-unfinishable download.
        if let Some(seq) = missing {
            halogen_webui_logging::warn!(
                episode_id = self.episode_id,
                seq,
                "partial chunk missing; discarding partial to restart from scratch"
            );
            discard_partial(&self.db, self.episode_id).await?;
            return Err(anyhow!(
                "partial chunk {seq} missing; download will restart"
            ));
        }
        let bag = web_sys::BlobPropertyBag::new();
        bag.set_type(&self.content_type);
        let blob = web_sys::Blob::new_with_blob_sequence_and_options(&parts, &bag)
            .map_err(|e| anyhow!("combine blob: {e:?}"))?;
        let value: JsValue = blob.into();

        // Commit the combined audio first, then drop the partial. If a crash lands
        // between the two, boot finds the id both committed and partial; hydrate
        // prefers committed (`Downloaded`) and the stale partial is reclaimable.
        let (tx, store) = one_store_tx(&self.db, STORE, TransactionMode::ReadWrite)?;
        store
            .put(&value, Some(&key(self.episode_id)))
            .map_err(idb_err("put"))?
            .await
            .map_err(idb_err("put (quota?)"))?;
        commit_tx(tx).await?;

        discard_partial(&self.db, self.episode_id).await?;
        Ok(())
    }
}
