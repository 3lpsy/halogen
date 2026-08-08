//! Device-download pipeline: the detached task that fetches an episode's audio to
//! the local media store, plus its resumable/retrying chunked + streaming
//! strategies and progress reporting.
//!
//! These are free functions (no `SyncService` access) — the worker spawns
//! [`device_download_task`] and stays the single `EpisodeState` writer; the megabytes
//! never ride through the Debug-logged `Command` enum.

use std::collections::BTreeMap;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use futures::stream::FuturesUnordered;
use halogen_api::{ApiClient, ApiError};
use halogen_wire::{DownloadStatus, EpisodeInclude};

use super::{Command, sleep_secs};
use halogen_ui_logging::{debug, warn};
#[cfg(target_arch = "wasm32")]
use halogen_ui_platform::weblock;
use halogen_ui_svc_media::{MediaStore, MediaWriter};

/// How long a device download waits for the SERVER to finish fetching its copy
/// before giving up (tries × interval ≈ 2 minutes).
const DEVICE_POLL_TRIES: u32 = 40;
const DEVICE_POLL_INTERVAL_SECS: u64 = 3;
/// Consecutive transport-error polls that mean "offline" rather than "server is
/// slow to fetch": bail early with an offline message instead of polling the full
/// `DEVICE_POLL_TRIES` budget against an unreachable server.
const DEVICE_POLL_OFFLINE_GIVEUP: u32 = 5;

/// Detached device-download task (spawned by `start_device_download`).
///
/// Phase 1 — wait for the server copy: `download_status` only changes on the
/// client via `CacheEpisodes` hand-backs from list revalidations, so this task
/// polls `get_episode` itself (the worker would otherwise never observe the
/// flip). Each fresh body is sent back as `CacheEpisodes`, so the server-status
/// icons update for free while we wait.
///
/// Phase 2 — pull the bytes (cookie-authed) and write them to the media store
/// HERE, then report only the outcome: the worker stays the single *EpisodeState*
/// writer, and megabytes never ride through the Debug-logged `Command` enum.
pub(super) async fn device_download_task(
    episode_id: i32,
    lock_name: String,
    offline: std::rc::Rc<std::cell::Cell<bool>>,
    api: ApiClient,
    media: std::rc::Rc<dyn MediaStore>,
    tx: UnboundedSender<Command>,
    mut server_ready: bool,
    chunk_bytes: Option<u64>,
    parallelism: u8,
) {
    let send = |result: Result<(), String>| {
        let _ = tx.unbounded_send(Command::DeviceFetchComplete { episode_id, result });
    };

    // Web: one download of an episode per origin at a time. Every tab (and the
    // sync worker inside each) shares the SAME IndexedDB chunk keyspace for
    // this episode, so two concurrent writers interleave chunks with
    // independent sequence counters and the first commit assembles corrupt
    // audio while the UI shows Downloaded. Held for the whole task;
    // auto-released if this tab dies mid-download, so the other tab's retry
    // proceeds. `Unsupported` (no Web Locks API) proceeds unguarded — the
    // pre-lock status quo. Native: one process owns the media store, no lock.
    #[cfg(target_arch = "wasm32")]
    let _cross_tab_lock = match weblock::try_acquire(&lock_name).await {
        weblock::TryLock::Busy => {
            return send(Err(
                "this episode is already downloading in another tab or window".into(),
            ));
        }
        weblock::TryLock::Acquired(guard) => Some(guard),
        weblock::TryLock::Unsupported => None,
    };
    #[cfg(not(target_arch = "wasm32"))]
    let _ = &lock_name;

    if !server_ready {
        // A run of back-to-back transport errors means the server is unreachable
        // (offline), not slow to fetch. Bail early with an offline-flavoured message
        // rather than burning the whole ~2-min budget on a dead link and then
        // reporting a misleading "server download timed out".
        let mut consecutive_errors: u32 = 0;
        for _ in 0..DEVICE_POLL_TRIES {
            // Manual "Go Offline" mid-wait: the user turned networking off —
            // stop polling the server instead of continuing for the budget.
            if offline.get() {
                return send(Err("cancelled — you went offline".into()));
            }
            // Include Playback so re-caching this body keeps the embedded cursor.
            match api
                .get_episode(episode_id, &[EpisodeInclude::Playback])
                .await
            {
                Ok(ep) => {
                    consecutive_errors = 0;
                    let status = ep.download_status.clone();
                    let _ = tx.unbounded_send(Command::CacheEpisodes { episodes: vec![ep] });
                    match status {
                        DownloadStatus::Downloaded => {
                            server_ready = true;
                            break;
                        }
                        // Any terminal failure: stop waiting and surface the error
                        // rather than polling until the budget runs out.
                        DownloadStatus::DownloadError
                        | DownloadStatus::DownloadUnauthorized
                        | DownloadStatus::DownloadRemoteNotFound
                        | DownloadStatus::DownloadBroken => {
                            return send(Err("the server couldn't fetch the episode".into()));
                        }
                        _ => {}
                    }
                }
                // Transient (offline blip): keep polling, but give up once enough
                // polls in a row fail — that's an unreachable server, not a slow one.
                Err(e) => {
                    consecutive_errors += 1;
                    warn!(episode_id, error = %e, consecutive_errors, "device download: poll failed");
                    if consecutive_errors >= DEVICE_POLL_OFFLINE_GIVEUP {
                        return send(Err(
                            "you appear to be offline — can't reach the server to download".into(),
                        ));
                    }
                }
            }
            sleep_secs(DEVICE_POLL_INTERVAL_SECS).await;
        }
        if !server_ready {
            // Budget exhausted. If the tail of the loop was nothing but transport
            // errors, the server's unreachable — report that instead of a download
            // timeout (which implies a reachable-but-slow server).
            if consecutive_errors > 0 {
                return send(Err(
                    "you appear to be offline — can't reach the server to download".into(),
                ));
            }
            return send(Err("timed out waiting for the server download".into()));
        }
    }

    // Phase 2: resumable, retrying, chunked download with progress. On failure we
    // KEEP the durable partial (it's invisible to `audio_url`/`list_ids`, so no
    // unplayable bytes are exposed) so the next attempt — a manual retry or the
    // boot-time `resume_partial_downloads` — continues from where this left off.
    // The only purge is the corruption-restart inside `download_audio`.
    if offline.get() {
        return send(Err("cancelled — you went offline".into()));
    }
    let result = download_audio(
        episode_id,
        &offline,
        &api,
        &media,
        &tx,
        chunk_bytes,
        parallelism,
    )
    .await;
    send(result);
}

/// Per-chunk attempts before the whole download gives up.
const DEVICE_CHUNK_RETRIES: u32 = 5;
const DEVICE_BACKOFF_INITIAL_MS: u32 = 500;
const DEVICE_BACKOFF_MAX_MS: u32 = 8_000;

/// Throttled progress publish: emit a `DeviceFetchProgress` only when the
/// whole-percent figure changes (≤100 publishes per download). `last_pct` is the
/// last value sent (`-1` = none yet); updated in place. No-op when `total` is
/// unknown or zero.
fn publish_device_progress(
    tx: &UnboundedSender<Command>,
    episode_id: i32,
    downloaded: u64,
    total: Option<u64>,
    last_pct: &mut i32,
) {
    if let Some(t) = total.filter(|t| *t > 0) {
        let pct = ((downloaded * 100) / t).min(100) as i32;
        if pct != *last_pct {
            *last_pct = pct;
            let _ = tx.unbounded_send(Command::DeviceFetchProgress {
                episode_id,
                percent: pct as u8,
            });
        }
    }
}

/// A chunked-download failure. `OffsetMismatch` is kept apart from the plain
/// message errors because it has a dedicated recovery: the server (or a proxy in
/// front of it) ignored the requested `Range` and served bytes from a different
/// offset — those bytes must never be written at the requested position, and the
/// download restarts from scratch instead (see [`download_audio_chunked`]).
#[derive(Clone, Debug)]
enum ChunkError {
    /// The response body starts at `served_from`, not the `requested` offset.
    OffsetMismatch { requested: u64, served_from: u64 },
    /// Any other failure, already formatted for the UI.
    Other(String),
}

impl ChunkError {
    fn into_message(self) -> String {
        match self {
            ChunkError::OffsetMismatch {
                requested,
                served_from,
            } => format!(
                "server ignored the requested byte range (asked for offset {requested}, served {served_from})"
            ),
            ChunkError::Other(msg) => msg,
        }
    }
}

/// Tag a chunk fetch with its start offset so out-of-order completions (parallel
/// mode) can be reordered into a contiguous write stream. Single source for the
/// in-flight future type, so a `FuturesUnordered` of these stays homogeneous.
async fn fetch_chunk_at(
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    episode_id: i32,
    start: u64,
    end: u64,
) -> (u64, Result<halogen_api::AudioChunk, ChunkError>) {
    (
        start,
        fetch_full_range(offline, api, episode_id, start, end).await,
    )
}

/// Fetch the COMPLETE byte range `[start, end]` (inclusive), re-requesting the
/// remainder if the server answers a 206 with fewer bytes than asked for.
///
/// Phase B's reorder buffer is keyed by the requested `start`, but the write cursor
/// (`next_write`) advances by the bytes actually written. If a chunk came back short
/// the two would diverge — `next_write` would land between buffer keys and the
/// assembly would stall ("download stalled"). Guaranteeing each chunk is its full
/// requested span keeps that invariant. A zero-byte response for a range the server
/// should still hold is a genuine error (rather than an infinite re-request loop).
async fn fetch_full_range(
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    episode_id: i32,
    start: u64,
    end: u64,
) -> Result<halogen_api::AudioChunk, ChunkError> {
    let want = (end - start + 1) as usize;
    let mut acc: Vec<u8> = Vec::with_capacity(want);
    let mut total: Option<u64> = None;
    let mut content_type: Option<String> = None;
    while acc.len() < want {
        let next = start + acc.len() as u64;
        let chunk = fetch_audio_chunk_retrying(offline, api, episode_id, next, end).await?;
        if chunk.total.is_some() {
            total = chunk.total;
        }
        if content_type.is_none() {
            content_type = chunk.content_type;
        }
        if chunk.bytes.is_empty() {
            // The server returned nothing for a sub-range it should still serve —
            // surface it rather than spin re-requesting the same offset forever.
            return Err(ChunkError::Other(format!(
                "short range read for episode {episode_id}: got {} of {want} bytes for [{start},{end}]",
                acc.len()
            )));
        }
        acc.extend_from_slice(&chunk.bytes);
    }
    // Defensive: a server that over-serves the range can't desync the write cursor.
    acc.truncate(want);
    Ok(halogen_api::AudioChunk {
        bytes: acc,
        total,
        // Every sub-request above was verified to serve from its requested offset,
        // so the assembled chunk genuinely starts at `start`.
        served_from: start,
        content_type,
    })
}

/// Bytes accumulated before a flush to storage in the streaming (no-chunking) path.
/// Caps the in-memory buffer and keeps the per-write storage-transaction count sane
/// (a browser body stream arrives in many small pieces).
const STREAM_FLUSH_BYTES: usize = 4 * 1024 * 1024;

/// Download an episode's audio to storage, picking the strategy from `chunk_bytes`:
/// `Some(len)` → ranged chunks (up to `parallelism` concurrent); `None` → no
/// chunking, one streamed request. Both resume from a staged partial and commit only
/// a verified-complete file.
async fn download_audio(
    episode_id: i32,
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    media: &std::rc::Rc<dyn MediaStore>,
    tx: &UnboundedSender<Command>,
    chunk_bytes: Option<u64>,
    parallelism: u8,
) -> Result<(), String> {
    match chunk_bytes {
        None => download_audio_streaming(episode_id, offline, api, media, tx).await,
        Some(chunk_len) => {
            download_audio_chunked(episode_id, offline, api, media, tx, chunk_len, parallelism)
                .await
        }
    }
}

/// Download an episode's audio in resumable ranged chunks and stream it to storage,
/// reporting progress. A chunk that fails is retried with exponential backoff
/// (only the dropped bytes are re-requested, not the whole file) and stored
/// chunks are kept — so a flaky connection still makes forward progress.
///
/// `chunk_len` is the per-request size. `parallelism` is how many chunks are
/// fetched concurrently within this one download (`1` = sequential); it only
/// applies once the total size is known.
///
/// Resumes a partial left by an earlier interrupted attempt (page refresh / app
/// restart): the staged byte offset comes from [`MediaStore::partial`], and the
/// download simply starts there. If the resumed partial's server-side total no
/// longer matches (the server copy changed under us), the partial is discarded and
/// the download restarts from byte 0. Chunk *boundaries* are not persisted — only
/// the byte offset — so resuming with a different configured chunk size is safe.
///
/// Writes always land in byte order regardless of `parallelism`: completed-ahead
/// chunks wait in a small reorder buffer until the writer reaches their offset, so
/// the append-only [`MediaWriter`] contract is preserved. The writer commits only
/// after a complete download; on any error this returns `Err` and the
/// (uncommitted) writer is dropped, leaving the durable partial in place to resume.
/// A short read is refused outright (the final total check), so a truncated
/// download can never masquerade as a complete one.
///
/// Every chunk response is verified to start at the offset it was requested at
/// (`served_from`, from `Content-Range`). One mismatch — a proxy that ignored the
/// `Range` and answered from byte 0 — discards the partial and re-runs the whole
/// attempt from scratch: a from-byte-0 request is always safe to honor literally.
/// If the from-scratch attempt ALSO gets mis-offset bytes the download fails with
/// an error (no retry loop against a peer that never honors ranges).
async fn download_audio_chunked(
    episode_id: i32,
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    media: &std::rc::Rc<dyn MediaStore>,
    tx: &UnboundedSender<Command>,
    chunk_len: u64,
    parallelism: u8,
) -> Result<(), String> {
    match download_audio_chunked_attempt(
        episode_id,
        offline,
        api,
        media,
        tx,
        chunk_len,
        parallelism,
    )
    .await
    {
        Err(ChunkError::OffsetMismatch {
            requested,
            served_from,
        }) => {
            warn!(
                episode_id,
                requested,
                served_from,
                "chunk served from the wrong offset; discarding partial and restarting from scratch"
            );
            let _ = media.remove_audio(episode_id).await;
            match download_audio_chunked_attempt(
                episode_id,
                offline,
                api,
                media,
                tx,
                chunk_len,
                parallelism,
            )
            .await
            {
                // The from-scratch retry ALSO got mis-offset bytes: the server
                // (or a proxy) never honors ranges, so chunking can't work —
                // fall back to the STREAMING strategy, which handles a
                // 200-from-byte-0 correctly with bounded memory (the chunked
                // client caps each buffered body, so this path no longer
                // succeeds by accident via a whole-file buffer). Never commits
                // stitched bytes and never restart-loops.
                Err(ChunkError::OffsetMismatch { .. }) => {
                    warn!(
                        episode_id,
                        "server never honors byte ranges; falling back to streaming download"
                    );
                    let _ = media.remove_audio(episode_id).await;
                    download_audio_streaming(episode_id, offline, api, media, tx).await
                }
                other => other.map_err(ChunkError::into_message),
            }
        }
        other => other.map_err(ChunkError::into_message),
    }
}

/// One pass of the chunked download (resume detection → Phase A/B′/B → commit).
/// Split from [`download_audio_chunked`] so an [`ChunkError::OffsetMismatch`] can
/// discard the partial and re-run the whole pass from scratch exactly once.
async fn download_audio_chunked_attempt(
    episode_id: i32,
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    media: &std::rc::Rc<dyn MediaStore>,
    tx: &UnboundedSender<Command>,
    chunk_len: u64,
    parallelism: u8,
) -> Result<(), ChunkError> {
    // Resume from a durable partial when one is staged (offset > 0).
    let resume = media
        .partial(episode_id)
        .await
        .ok()
        .flatten()
        .filter(|p| p.downloaded > 0);
    let mut downloaded: u64 = resume.as_ref().map(|p| p.downloaded).unwrap_or(0);
    let mut total: Option<u64> = resume.as_ref().and_then(|p| p.total);
    let mut last_pct: i32 = -1;
    // When resuming, open the (appending) writer up front so it keeps the
    // partial's content type; otherwise open lazily from the first response so the
    // writer carries the real content type and a zero-byte/404 never opens one.
    let mut writer: Option<Box<dyn MediaWriter>> = match resume.as_ref() {
        Some(info) => {
            debug!(episode_id, downloaded, "resuming device download");
            Some(
                media
                    .open_writer(episode_id, info.content_type.as_deref(), info.total, true)
                    .await
                    .map_err(|e| ChunkError::Other(e.to_string()))?,
            )
        }
        None => None,
    };
    // Set on the first chunk after a resume: validate the partial still matches the
    // server copy before trusting the bytes already on disk.
    let mut verifying_resume = resume.is_some();

    // The end offset for a request starting at `start`.
    let chunk_end = |start: u64| -> u64 { start.saturating_add(chunk_len).saturating_sub(1) };

    // Set when the first written chunk came back FULL (== chunk_len) with no
    // server-reported total: the server is range-serving without advertising a
    // total, so the final length check can't catch truncation — Phase B′ keeps
    // pulling until a short/empty chunk proves EOF.
    let mut full_untotaled_first = false;

    // ── Phase A: the first chunk, fetched alone ──────────────────────────────
    // Handles resume verification, lazy writer-open, and learning `total` /
    // content type before any parallel fan-out. Iterates at most twice (a second
    // pass only when a resume mismatch or an oversized staged partial restarts
    // from byte 0).
    loop {
        if let Some(t) = total {
            if downloaded == t {
                break; // resume partial was already complete
            }
            if downloaded > t {
                // An impossible partial — more bytes staged than the file holds
                // (e.g. an earlier run appended mis-offset bytes). Resuming can
                // never complete it and the final length check would refuse it
                // forever, so discard it and start over.
                warn!(
                    episode_id,
                    downloaded,
                    total = t,
                    "staged partial larger than the server copy; restarting"
                );
                writer = None; // drop the appending writer (keeps the on-disk partial)
                let _ = media.remove_audio(episode_id).await; // then discard it
                downloaded = 0;
                total = None;
                verifying_resume = false;
                continue; // re-fetch from byte 0 as a fresh download
            }
        }
        let chunk =
            fetch_audio_chunk_retrying(offline, api, episode_id, downloaded, chunk_end(downloaded))
                .await?;

        if verifying_resume {
            verifying_resume = false;
            // Accept the resume only if the server still range-serves the SAME
            // file (total matches what we staged, or we hadn't recorded a total).
            match chunk.total {
                Some(ct) if total.is_none_or(|st| st == ct) => total = Some(ct),
                other => {
                    warn!(
                        episode_id,
                        staged = ?total,
                        server = ?other,
                        "resume partial no longer matches server copy; restarting"
                    );
                    writer = None; // drop the appending writer (keeps the on-disk partial)
                    let _ = media.remove_audio(episode_id).await; // then discard it
                    downloaded = 0;
                    total = None;
                    continue; // re-fetch from byte 0 as a fresh download
                }
            }
        }

        if writer.is_none() {
            total = chunk.total;
            writer = Some(
                media
                    .open_writer(
                        episode_id,
                        chunk.content_type.as_deref(),
                        chunk.total,
                        false,
                    )
                    .await
                    .map_err(|e| ChunkError::Other(e.to_string()))?,
            );
        }
        if chunk.bytes.is_empty() {
            break; // server has no more to give
        }
        writer
            .as_mut()
            .expect("writer opened above")
            .write(&chunk.bytes)
            .await
            .map_err(|e| ChunkError::Other(e.to_string()))?;
        downloaded += chunk.bytes.len() as u64;
        full_untotaled_first = total.is_none() && chunk.bytes.len() as u64 == chunk_len;
        publish_device_progress(tx, episode_id, downloaded, total, &mut last_pct);
        break;
    }

    // ── Phase B′: unknown-total continuation ─────────────────────────────────
    // The server range-served a full first chunk but advertised no total (no
    // Content-Range), so neither Phase B nor the final length check below would
    // run. Pull sequential ranges until a short/empty chunk marks EOF, so a sliced-
    // but-untotaled response can never commit as a truncated "complete" file. If
    // the server starts reporting a total mid-stream, hand off to the windowed
    // Phase B below.
    while full_untotaled_first {
        let chunk =
            fetch_audio_chunk_retrying(offline, api, episode_id, downloaded, chunk_end(downloaded))
                .await?;
        if chunk.total.is_some() {
            total = chunk.total;
        }
        if chunk.bytes.is_empty() {
            break;
        }
        let n = chunk.bytes.len() as u64;
        writer
            .as_mut()
            .expect("writer opened in Phase A")
            .write(&chunk.bytes)
            .await
            .map_err(|e| ChunkError::Other(e.to_string()))?;
        downloaded += n;
        publish_device_progress(tx, episode_id, downloaded, total, &mut last_pct);
        full_untotaled_first = total.is_none() && n == chunk_len;
    }

    // ── Phase B: remaining chunks, up to `parallelism` in flight ─────────────
    // Runs when the total is known and there's more to fetch (a `None` total means
    // the server ignored Range and Phase A already pulled the whole body). With
    // `parallelism == 1` this is a plain sequential loop.
    if let Some(t) = total
        && downloaded < t
    {
        let window = parallelism.max(1) as usize;
        let mut next_write = downloaded; // next byte offset that must be written
        let mut next_fetch = downloaded; // next byte offset to request
        let mut received = downloaded; // bytes pulled (any order) — drives progress
        let writer = writer.as_mut().expect("writer opened in Phase A");
        let mut inflight = FuturesUnordered::new();
        let mut buffer: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
        // Set when a chunk fetch fails for good: stop issuing new requests and write
        // out everything contiguously available BELOW the gap before surfacing the
        // error, so a flaky link still persists the bytes it did pull (rather than
        // discarding a whole window's worth that completed ahead of the failure).
        let mut fetch_error: Option<ChunkError> = None;

        while next_write < t {
            // Keep the window full, counting BOTH in-flight requests AND
            // completed-but-unwritten chunks against it — so a slow `next_write`
            // chunk can't let later arrivals pile up past `window × chunk` bytes
            // (the bound the UI caption promises). Refilled at the top of the loop
            // so draining the buffer re-seeds the window. Steady state keeps
            // `window` requests in flight; only a stall throttles issuance. Once a
            // fetch has failed we stop adding work and just flush what we have.
            while fetch_error.is_none() && inflight.len() + buffer.len() < window && next_fetch < t
            {
                let start = next_fetch;
                let end = chunk_end(start).min(t - 1);
                next_fetch = end + 1;
                inflight.push(fetch_chunk_at(offline, api, episode_id, start, end));
            }

            // Drain completions until the chunk we need to write next is buffered.
            // Each completion moves a chunk in-flight → buffer, leaving the
            // outstanding total unchanged, so the next refill happens after a write
            // frees a slot (back at the top). `next_write`'s chunk is always issued
            // before any later one, so while it's still in flight `inflight` is
            // non-empty and `.next()` yields.
            while !buffer.contains_key(&next_write) {
                let Some((start, res)) = inflight.next().await else {
                    // Nothing left in flight and the chunk at `next_write` never
                    // arrived: either a fetch failed (surface that) or a short read
                    // broke alignment. Either way the partial below `next_write` is
                    // already persisted; resume re-aligns sequentially via Phase A.
                    return Err(fetch_error.clone().unwrap_or_else(|| {
                        ChunkError::Other(format!("download stalled at {next_write}/{t} bytes"))
                    }));
                };
                match res {
                    // The server's copy changed size mid-download (a concurrent
                    // re-download): this chunk's bytes are from a different file, so
                    // don't trust them. Treat it like a failure — drain & write the
                    // consistent chunks below the gap, then bail; resume's Phase A
                    // total-check then discards the stale partial and restarts clean.
                    Ok(chunk) if chunk.total.is_some_and(|ct| ct != t) => {
                        fetch_error.get_or_insert_with(|| {
                            ChunkError::Other(format!(
                                "server copy changed during download (was {t}, now {})",
                                chunk.total.unwrap_or(0)
                            ))
                        });
                    }
                    Ok(chunk) => {
                        received += chunk.bytes.len() as u64;
                        // Progress reflects bytes RECEIVED (any order), not just
                        // bytes written in order — otherwise the bar freezes behind
                        // a straggler while later chunks are already in hand.
                        publish_device_progress(tx, episode_id, received, total, &mut last_pct);
                        buffer.insert(start, chunk.bytes);
                    }
                    // Remember the failure but keep draining: chunks already in
                    // `buffer` below the gap still get written before we bail.
                    Err(e) => {
                        fetch_error.get_or_insert(e);
                    }
                }
            }
            let bytes = buffer.remove(&next_write).expect("present per loop guard");
            if bytes.is_empty() {
                break; // server returned nothing for an in-range request
            }
            writer
                .write(&bytes)
                .await
                .map_err(|e| ChunkError::Other(e.to_string()))?;
            downloaded += bytes.len() as u64;
            next_write += bytes.len() as u64;
        }
    }

    if let Some(t) = total
        && downloaded != t
    {
        return Err(ChunkError::Other(format!(
            "incomplete download ({downloaded}/{t} bytes)"
        )));
    }
    let Some(mut writer) = writer.filter(|_| downloaded > 0) else {
        return Err(ChunkError::Other("empty download".into()));
    };
    // Bytes become the stored audio only here, after a verified-complete download.
    writer
        .commit()
        .await
        .map_err(|e| ChunkError::Other(e.to_string()))
}

/// Download an episode's audio with NO chunking: a single open-ended request whose
/// body is streamed to storage in `STREAM_FLUSH_BYTES` flushes, so the whole (often
/// 100+ MB) file is never buffered in memory and the download stays resumable from
/// the last flushed offset. There is no parallelism and no reorder buffer — one
/// request, written in arrival order. A single response means `total` can't shift
/// mid-stream, so there's no mid-download re-verification to do (only the resume
/// check below).
async fn download_audio_streaming(
    episode_id: i32,
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    media: &std::rc::Rc<dyn MediaStore>,
    tx: &UnboundedSender<Command>,
) -> Result<(), String> {
    // `true` after a resume mismatch forces a single from-byte-0 restart.
    let mut from_scratch = false;
    loop {
        let resume = if from_scratch {
            None
        } else {
            media
                .partial(episode_id)
                .await
                .ok()
                .flatten()
                .filter(|p| p.downloaded > 0)
        };
        let mut downloaded: u64 = resume.as_ref().map(|p| p.downloaded).unwrap_or(0);
        let staged_total = resume.as_ref().and_then(|p| p.total);
        let staged_ct = resume.as_ref().and_then(|p| p.content_type.clone());
        if resume.is_some() {
            debug!(
                episode_id,
                downloaded, "resuming device download (streaming)"
            );
        }

        let mut audio = api
            .download_audio_stream(episode_id, downloaded)
            .await
            .map_err(|e| e.to_string())?;

        // A resume is valid only if the server range-served from exactly where we
        // left off AND the size still matches; otherwise (range ignored → served
        // from 0, or the copy changed) discard the partial and restart from byte 0.
        if resume.is_some() {
            let size_ok =
                audio.total.is_none() || staged_total.is_none() || audio.total == staged_total;
            if audio.served_from != downloaded || !size_ok {
                warn!(
                    episode_id,
                    staged_offset = downloaded,
                    served_from = audio.served_from,
                    staged_total = ?staged_total,
                    server_total = ?audio.total,
                    "streaming resume no longer matches server copy; restarting"
                );
                let _ = media.remove_audio(episode_id).await;
                from_scratch = true;
                continue;
            }
        }

        let total = audio.total.or(staged_total);
        let content_type = if resume.is_some() {
            staged_ct
        } else {
            audio.content_type.clone()
        };
        let mut writer = media
            .open_writer(episode_id, content_type.as_deref(), total, resume.is_some())
            .await
            .map_err(|e| e.to_string())?;

        // Accumulate the body's (many, small) pieces and flush in bounded batches —
        // streaming to storage so the wasm heap only ever holds one batch.
        let mut last_pct: i32 = -1;
        let mut batch: Vec<u8> = Vec::with_capacity(STREAM_FLUSH_BYTES);
        while let Some(piece) = audio.body.next().await {
            if offline.get() {
                return Err("cancelled — you went offline".to_string());
            }
            let bytes = piece.map_err(|e| e.to_string())?;
            batch.extend_from_slice(&bytes);
            if batch.len() >= STREAM_FLUSH_BYTES {
                writer.write(&batch).await.map_err(|e| e.to_string())?;
                downloaded += batch.len() as u64;
                batch.clear();
                publish_device_progress(tx, episode_id, downloaded, total, &mut last_pct);
            }
        }
        if !batch.is_empty() {
            writer.write(&batch).await.map_err(|e| e.to_string())?;
            downloaded += batch.len() as u64;
            publish_device_progress(tx, episode_id, downloaded, total, &mut last_pct);
        }

        if let Some(t) = total
            && downloaded != t
        {
            return Err(format!("incomplete download ({downloaded}/{t} bytes)"));
        }
        if downloaded == 0 {
            return Err("empty download".into());
        }
        // Bytes become the stored audio only here, after a verified-complete download.
        return writer.commit().await.map_err(|e| e.to_string());
    }
}

/// Fetch one chunk with bounded exponential-backoff retries; the error is the last
/// attempt's message. Every response is verified to start at the requested `start`
/// offset — a server/proxy that ignored the `Range` (served from elsewhere, usually
/// byte 0) yields [`ChunkError::OffsetMismatch`] immediately (no retries: a peer
/// that ignores ranges won't honor the next one either), so mis-offset bytes can
/// never be written at the requested position.
async fn fetch_audio_chunk_retrying(
    offline: &std::rc::Rc<std::cell::Cell<bool>>,
    api: &ApiClient,
    episode_id: i32,
    start: u64,
    end: u64,
) -> Result<halogen_api::AudioChunk, ChunkError> {
    let mut delay = DEVICE_BACKOFF_INITIAL_MS;
    let mut last_err = String::new();
    for attempt in 1..=DEVICE_CHUNK_RETRIES {
        // The single choke point every chunked path fetches through — one
        // check here covers Phase A/B'/B and the full-range assembler. Manual
        // "Go Offline" must stop the transfer, not just the sync loop.
        if offline.get() {
            return Err(ChunkError::Other("cancelled — you went offline".into()));
        }
        match api.download_audio_range(episode_id, start, end).await {
            Ok(chunk) if chunk.served_from != start => {
                warn!(
                    episode_id,
                    start,
                    served_from = chunk.served_from,
                    "audio chunk served from the wrong offset"
                );
                return Err(ChunkError::OffsetMismatch {
                    requested: start,
                    served_from: chunk.served_from,
                });
            }
            Ok(chunk) => return Ok(chunk),
            Err(e) => {
                // Permanent failures (auth / gone) won't succeed on retry — fail
                // fast instead of burning the whole backoff budget on them.
                let permanent = matches!(&e, ApiError::Server { status, .. } if matches!(*status, 401 | 403 | 404 | 410));
                last_err = e.to_string();
                warn!(episode_id, start, attempt, permanent, error = %last_err, "audio chunk failed");
                if permanent {
                    break;
                }
                if attempt < DEVICE_CHUNK_RETRIES {
                    halogen_ui_platform::time::sleep_ms(delay).await;
                    delay = (delay * 2).min(DEVICE_BACKOFF_MAX_MS);
                }
            }
        }
    }
    Err(ChunkError::Other(last_err))
}

// Chunked-download behavior tests against a real (loopback) HTTP server whose
// `Range` handling is scripted per test — the point is the client's reaction to a
// server/proxy that ignores the requested byte offset, which no pure-function
// test can cover. Native-only: the download path runs identically on both
// targets, and the harness needs tokio + a TCP listener.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use futures::channel::mpsc::unbounded;
    use halogen_api::ApiClient;
    use halogen_ui_svc_media::PartialInfo;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use url::Url;

    use super::{Command, MediaStore, MediaWriter, download_audio_chunked};

    /// A never-tripped manual-offline flag for tests (downloads run to completion).
    fn online_flag() -> Rc<Cell<bool>> {
        Rc::new(Cell::new(false))
    }

    /// How the mock server treats the `Range` header of nonzero-offset requests
    /// (a request from byte 0 is always answered from byte 0 — honoring and
    /// ignoring are indistinguishable there).
    #[derive(Clone, Copy)]
    enum RangeMode {
        /// Honor every `Range`: `206` + `Content-Range` + the requested slice.
        Honor,
        /// Ignore the `Range` of every nonzero-offset request: answer `200` with
        /// the whole file from byte 0 (the misbehaving-proxy failure mode).
        IgnoreNonzero,
        /// Ignore the `Range` of the FIRST nonzero-offset request, then honor —
        /// one bad answer, after which a restarted download can succeed.
        IgnoreNonzeroOnce,
    }

    /// Spawn a one-endpoint audio server on a loopback port; returns its base URL
    /// and the log of requested start offsets, in arrival order.
    async fn spawn_audio_server(file: Vec<u8>, mode: RangeMode) -> (Url, Arc<Mutex<Vec<u64>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = Url::parse(&format!("http://{}", listener.local_addr().expect("addr")))
            .expect("base url");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let ignored_once = Arc::new(Mutex::new(false));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let file = file.clone();
                let log = log.clone();
                let ignored_once = ignored_once.clone();
                tokio::spawn(async move {
                    // Read the request head (GET — no body follows).
                    let mut head = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => head.extend_from_slice(&buf[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let total = file.len() as u64;
                    // `Range: bytes=<start>-<end>` (the chunked client always
                    // sends a bounded range).
                    let (start, end) = head
                        .lines()
                        .find_map(|l| {
                            let (name, v) = l.split_once(':')?;
                            name.trim()
                                .eq_ignore_ascii_case("range")
                                .then(|| v.trim().to_string())
                        })
                        .as_deref()
                        .and_then(|r| r.strip_prefix("bytes="))
                        .and_then(|spec| spec.split_once('-'))
                        .map(|(s, e)| {
                            let s = s.parse::<u64>().unwrap_or(0);
                            let e = e
                                .parse::<u64>()
                                .unwrap_or(total.saturating_sub(1))
                                .min(total.saturating_sub(1));
                            (s, e)
                        })
                        .unwrap_or((0, total.saturating_sub(1)));
                    log.lock().expect("request log").push(start);
                    let ignore_range = start > 0
                        && match mode {
                            RangeMode::Honor => false,
                            RangeMode::IgnoreNonzero => true,
                            RangeMode::IgnoreNonzeroOnce => {
                                let mut done = ignored_once.lock().expect("once flag");
                                !std::mem::replace(&mut *done, true)
                            }
                        };
                    let response = if start >= total {
                        b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                    } else if ignore_range {
                        // The misbehaving proxy: 200, whole file from byte 0.
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                        )
                        .into_bytes();
                        r.extend_from_slice(&file);
                        r
                    } else {
                        let body = &file[start as usize..=end as usize];
                        let mut r = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mpeg\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(body);
                        r
                    };
                    let _ = sock.write_all(&response).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (base, requests)
    }

    /// Shared state behind the in-memory media store: staged (uncommitted)
    /// bytes, the committed copy, and a discard counter.
    #[derive(Default)]
    struct MemMedia {
        staged: RefCell<Vec<u8>>,
        staged_total: RefCell<Option<u64>>,
        staged_content_type: RefCell<Option<String>>,
        committed: RefCell<Option<Vec<u8>>>,
        removed: Cell<usize>,
    }

    /// In-memory [`MediaStore`] over [`MemMedia`] (single-episode: the id is
    /// ignored — each test downloads exactly one episode).
    struct MemMediaStore(Rc<MemMedia>);

    #[async_trait(?Send)]
    impl MediaStore for MemMediaStore {
        async fn open_writer(
            &self,
            _episode_id: i32,
            content_type: Option<&str>,
            total: Option<u64>,
            resume: bool,
        ) -> anyhow::Result<Box<dyn MediaWriter>> {
            if !resume {
                self.0.staged.borrow_mut().clear();
            }
            *self.0.staged_total.borrow_mut() = total;
            *self.0.staged_content_type.borrow_mut() = content_type.map(str::to_string);
            Ok(Box::new(MemMediaWriter(self.0.clone())))
        }
        async fn audio_url(
            &self,
            _episode_id: i32,
        ) -> anyhow::Result<Option<halogen_ui_svc_media::LocalAudio>> {
            Ok(self
                .0
                .committed
                .borrow()
                .is_some()
                .then(|| halogen_ui_svc_media::LocalAudio {
                    url: "mem:audio".to_string(),
                    content_type: None,
                }))
        }
        async fn remove_audio(&self, _episode_id: i32) -> anyhow::Result<()> {
            self.0.staged.borrow_mut().clear();
            *self.0.staged_total.borrow_mut() = None;
            *self.0.committed.borrow_mut() = None;
            self.0.removed.set(self.0.removed.get() + 1);
            Ok(())
        }
        async fn list_ids(&self) -> anyhow::Result<Vec<i32>> {
            Ok(Vec::new())
        }
        async fn partial(&self, _episode_id: i32) -> anyhow::Result<Option<PartialInfo>> {
            let staged = self.0.staged.borrow();
            Ok((!staged.is_empty()).then(|| PartialInfo {
                downloaded: staged.len() as u64,
                total: *self.0.staged_total.borrow(),
                content_type: self.0.staged_content_type.borrow().clone(),
            }))
        }
        async fn list_partials(&self) -> anyhow::Result<Vec<i32>> {
            Ok(Vec::new())
        }
        async fn clear(&self) -> anyhow::Result<()> {
            self.0.staged.borrow_mut().clear();
            *self.0.staged_total.borrow_mut() = None;
            *self.0.committed.borrow_mut() = None;
            Ok(())
        }
    }

    struct MemMediaWriter(Rc<MemMedia>);

    #[async_trait(?Send)]
    impl MediaWriter for MemMediaWriter {
        async fn write(&mut self, chunk: &[u8]) -> anyhow::Result<()> {
            self.0.staged.borrow_mut().extend_from_slice(chunk);
            Ok(())
        }
        async fn commit(&mut self) -> anyhow::Result<()> {
            let bytes = std::mem::take(&mut *self.0.staged.borrow_mut());
            *self.0.committed.borrow_mut() = Some(bytes);
            *self.0.staged_total.borrow_mut() = None;
            Ok(())
        }
    }

    /// Run a `!Send` future (the download path holds `Rc`s) on a current-thread
    /// tokio runtime — reqwest and the mock server need a live reactor.
    fn run_local<F: Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(tokio::task::LocalSet::new().run_until(fut))
    }

    /// The 40-byte test file: distinct values, so any bytes written at the wrong
    /// offset change the committed content.
    fn test_file() -> Vec<u8> {
        (0u8..40).collect()
    }

    const CHUNK: u64 = 16;

    /// One chunk served from the wrong offset (a proxy ignored `Range` once)
    /// discards the partial and restarts the download from scratch — and the
    /// committed file is byte-for-byte correct, never a silent corrupt commit.
    #[test]
    fn chunked_offset_mismatch_restarts_from_scratch() {
        run_local(async {
            let file = test_file();
            let (base, requests) =
                spawn_audio_server(file.clone(), RangeMode::IgnoreNonzeroOnce).await;
            let api = ApiClient::new(base);
            let state = Rc::new(MemMedia::default());
            let media: Rc<dyn MediaStore> = Rc::new(MemMediaStore(state.clone()));
            let (tx, _rx) = unbounded::<Command>();

            download_audio_chunked(1, &online_flag(), &api, &media, &tx, CHUNK, 1)
                .await
                .expect("restarted download completes");

            assert_eq!(
                state.committed.borrow().as_deref(),
                Some(file.as_slice()),
                "committed bytes must match the server file exactly"
            );
            assert_eq!(state.removed.get(), 1, "the partial was discarded once");
            let starts = requests.lock().expect("request log").clone();
            // Attempt 1: 0 (honored), 16 (ignored → mismatch). Restart: 0, 16, 32.
            assert_eq!(starts, vec![0, 16, 0, 16, 32]);
        });
    }

    /// A server that NEVER honors nonzero ranges: after the one from-scratch
    /// restart also fails, the download falls back to the STREAMING strategy
    /// (bounded memory, handles 200-from-0 natively) and completes with
    /// byte-correct content. The invariants stand: no file stitched from
    /// mis-offset bytes is ever committed, and there is no restart loop.
    #[test]
    fn chunked_persistent_offset_mismatch_falls_back_to_streaming() {
        run_local(async {
            let file = test_file();
            let (base, _requests) =
                spawn_audio_server(file.clone(), RangeMode::IgnoreNonzero).await;
            let api = ApiClient::new(base);
            let state = Rc::new(MemMedia::default());
            let media: Rc<dyn MediaStore> = Rc::new(MemMediaStore(state.clone()));
            let (tx, _rx) = unbounded::<Command>();

            download_audio_chunked(1, &online_flag(), &api, &media, &tx, CHUNK, 1)
                .await
                .expect("streaming fallback completes the download");

            assert_eq!(
                state.committed.borrow().as_deref(),
                Some(file.as_slice()),
                "committed bytes must match the server file exactly"
            );
            assert_eq!(
                state.removed.get(),
                2,
                "both mis-offset partials were discarded (restart + fallback)"
            );
        });
    }

    /// A staged partial LARGER than the server file (an impossible resume — e.g.
    /// left behind by an earlier run that appended mis-offset bytes) is discarded
    /// and the download starts over instead of wedging forever on the
    /// incomplete-download check.
    #[test]
    fn chunked_resume_larger_than_total_discards_partial() {
        run_local(async {
            let file = test_file();
            let (base, requests) = spawn_audio_server(file.clone(), RangeMode::Honor).await;
            let api = ApiClient::new(base);
            let state = Rc::new(MemMedia::default());
            // Stage an oversized partial: 60 junk bytes against a 40-byte file.
            *state.staged.borrow_mut() = vec![0xAA; 60];
            *state.staged_total.borrow_mut() = Some(file.len() as u64);
            let media: Rc<dyn MediaStore> = Rc::new(MemMediaStore(state.clone()));
            let (tx, _rx) = unbounded::<Command>();

            download_audio_chunked(1, &online_flag(), &api, &media, &tx, CHUNK, 1)
                .await
                .expect("restarted download completes");

            assert_eq!(
                state.committed.borrow().as_deref(),
                Some(file.as_slice()),
                "committed bytes must match the server file exactly"
            );
            assert_eq!(
                state.removed.get(),
                1,
                "the oversized partial was discarded"
            );
            let starts = requests.lock().expect("request log").clone();
            // Never a request at the bogus offset 60 — a fresh 0/16/32 pull.
            assert_eq!(starts, vec![0, 16, 32]);
        });
    }
}
