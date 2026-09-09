//! On-demand server-side artwork cache; clients never fetch origin art directly. Persist successful paths and
//! negative-cache failures for NEGATIVE_TTL. Episode/podcast fallback permits one hop only. Per-key locks cover leaf
//! fetches, with DB rechecks, but never cross-entity fallback, avoiding deadlocks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tracing::{debug, warn};

use halogen_orm::{episode, podcast};

/// How long a *transient* failed art fetch (network blip, 5xx, timeout)
/// suppresses retries for that entity.
const NEGATIVE_TTL: Duration = Duration::from_secs(15 * 60);

/// How long a *permanent* failure (the origin isn't an image, is an SVG, 404s,
/// or is oversized) suppresses retries. Structurally-bad art won't fix itself on
/// the next render, so it's parked for a week instead of re-crawled every 15 min.
const PERMANENT_NEGATIVE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Longest edge (px) of the small art variant. List/mini-player thumbnails render at ~70 CSS px. 160 covers up
/// to ~2.3x DPR crisply (and 3x acceptably for a thumbnail this small) while roughly halving the on-the-wire
/// bytes vs. the old 256 — which Lighthouse flagged as ~2x oversized for the displayed size. Bump back toward
/// 192/256 if hi-DPI sharpness ever regresses; drop to 128 to trade more sharpness for fewer bytes.
const SMALL_MAX_DIM: u32 = 160;

/// Entities whose last fetch failed, with the failure instant + the TTL that
/// failure earns. Pruned lazily on insert; bounded by the number of distinct
/// broken entities.
static FAILED: LazyLock<Mutex<HashMap<String, (Instant, Duration)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Per-entity fetch locks. Pruned in [`fetch_lock`] to entries some resolver is
/// actively holding, so the map stays bounded by in-flight fetches rather than
/// growing once per entity ever resolved.
static FETCH_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Limit concurrent thumbnail decode/encode work to the core count. Each blocking image conversion holds one permit,
/// bounding memory and blocking-pool pressure during list scrolls.
static SMALL_GEN_LIMIT: LazyLock<Semaphore> = LazyLock::new(|| {
    // Conservative cap (≤4): bounds worst-case decode memory to
    // permits × SMALL_DECODE_MAX_ALLOC regardless of core count, since OOM is
    // about total bytes, not just job count. Generation is one-shot per row
    // (then the file is cached forever), so throughput here isn't critical.
    let permits = std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(2);
    Semaphore::new(permits)
});

/// Per-image decode memory ceiling. `image` decodes the whole source into RAM before downscaling, so a
/// feed-controlled origin (up to `MAX_ART_BYTES` compressed) can balloon to hundreds of MB of raster. Cap the
/// decoder's allocation so a bomb fails the decode (→ we fall back to serving the original, no thumbnail)
/// instead of OOM-ing the pod. With [`SMALL_GEN_LIMIT`] this bounds peak generation memory to permits × this.
const SMALL_DECODE_MAX_ALLOC: u64 = 128 * 1024 * 1024;

/// Shared, SSRF-guarded HTTP client for art fetches, memoized so the connection pool is reused across requests
/// (the UI requests art for every rendered row). SHORT total timeout — not `halogen_download`'s hour-scale
/// download client: art resolves synchronously inside a browser `<img>` request, so a dead CDN must fail in
/// seconds, not pin handlers for an hour.
static ART_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn art_client() -> reqwest::Client {
    ART_CLIENT
        .get_or_init(|| {
            halogen_net::guarded_client_builder()
                .user_agent(halogen_net::user_agent("podcast-art"))
                .timeout(Duration::from_secs(10))
                .build()
                .expect("build reqwest art client")
        })
        .clone()
}

fn recently_failed(key: &str) -> bool {
    FAILED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(key)
        .is_some_and(|(at, ttl)| at.elapsed() < *ttl)
}

fn mark_failed(key: &str, ttl: Duration) {
    let mut failed = FAILED.lock().unwrap_or_else(|p| p.into_inner());
    failed.retain(|_, (at, ttl)| at.elapsed() < *ttl);
    failed.insert(key.to_string(), (Instant::now(), ttl));
}

fn fetch_lock(key: &str) -> Arc<AsyncMutex<()>> {
    let mut locks = FETCH_LOCKS.lock().unwrap_or_else(|p| p.into_inner());
    // Drop locks no resolver currently holds: a `strong_count == 1` means only the
    // map references it (every active resolver keeps its own clone alive across the
    // fetch). Runs before the insert below, so the key we're about to hand out is
    // never pruned.
    locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    locks.entry(key.to_string()).or_default().clone()
}

/// Resolve (fetching + caching if needed) the artwork file for an episode.
/// `Ok(None)` = no artwork available. When `fallback_podcast_art` is set and the
/// episode has no usable art, falls back to the parent podcast's art.
pub async fn ensure_episode_art(
    dbc: &DatabaseConnection,
    episode_id: i32,
    media_root: &Path,
    fallback_podcast_art: bool,
) -> anyhow::Result<Option<PathBuf>> {
    debug!(episode_id, fallback_podcast_art, "resolving episode art");
    let Some(row) = episode::Entity::find_by_id(episode_id).one(dbc).await? else {
        anyhow::bail!("episode {episode_id} not found");
    };

    if let Some(cached) = existing(&row.art_file_path) {
        debug!(episode_id, path = %cached.display(), "episode art cache hit");
        return Ok(Some(cached));
    }

    if let Some(url) = row.art_url.clone() {
        let key = format!("episode_{episode_id}");
        if recently_failed(&key) {
            debug!(episode_id, "skipping art fetch (recent failure)");
        } else {
            let lock = fetch_lock(&key);
            let _guard = lock.lock().await;
            // Re-check after acquiring: a concurrent resolver may have cached
            // the file (or marked the failure) while we waited.
            if let Some(row2) = episode::Entity::find_by_id(episode_id).one(dbc).await?
                && let Some(cached) = existing(&row2.art_file_path)
            {
                return Ok(Some(cached));
            }
            if !recently_failed(&key) {
                match fetch_art(&url, media_root, &key).await {
                    Ok(path) => {
                        let mut update = episode::ActiveModel::from(row);
                        update.art_file_path = Set(Some(path.display().to_string()));
                        update.update(dbc).await?;
                        return Ok(Some(path));
                    }
                    Err(e) => {
                        mark_failed(&key, e.ttl());
                        warn!(episode_id, error = %e.error, permanent = e.permanent, "episode art fetch failed");
                    }
                }
            }
        }
    } else {
        debug!(episode_id, "episode has no art_url");
    }

    if fallback_podcast_art {
        // Routine: episodes commonly have no per-episode art and inherit the
        // podcast's. Logged at debug so it doesn't drown real warnings (an
        // episode-art-less feed otherwise emits one WARN per episode per list).
        debug!(
            episode_id,
            podcast_id = row.podcast_id,
            "episode art unavailable; falling back to podcast art"
        );
        // Cross-call with the fallback OFF so the two methods can't loop.
        return Box::pin(ensure_podcast_art(dbc, row.podcast_id, media_root, false)).await;
    }

    Ok(None)
}

/// Resolve (fetching + caching if needed) the artwork file for a podcast.
/// `Ok(None)` = no artwork available. When `fallback_episode_art` is set and the
/// podcast has no usable art, falls back to its latest-published episode's art.
pub async fn ensure_podcast_art(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    media_root: &Path,
    fallback_episode_art: bool,
) -> anyhow::Result<Option<PathBuf>> {
    debug!(podcast_id, fallback_episode_art, "resolving podcast art");
    let Some(row) = podcast::Entity::find_by_id(podcast_id).one(dbc).await? else {
        anyhow::bail!("podcast {podcast_id} not found");
    };

    if let Some(cached) = existing(&row.art_file_path) {
        debug!(podcast_id, path = %cached.display(), "podcast art cache hit");
        return Ok(Some(cached));
    }

    if let Some(url) = row.art_url.clone() {
        let key = format!("podcast_{podcast_id}");
        if recently_failed(&key) {
            debug!(podcast_id, "skipping art fetch (recent failure)");
        } else {
            let lock = fetch_lock(&key);
            let _guard = lock.lock().await;
            // Re-check after acquiring: a concurrent resolver may have cached
            // the file (or marked the failure) while we waited.
            if let Some(row2) = podcast::Entity::find_by_id(podcast_id).one(dbc).await?
                && let Some(cached) = existing(&row2.art_file_path)
            {
                return Ok(Some(cached));
            }
            if !recently_failed(&key) {
                match fetch_art(&url, media_root, &key).await {
                    Ok(path) => {
                        let mut update = podcast::ActiveModel::from(row);
                        update.art_file_path = Set(Some(path.display().to_string()));
                        update.update(dbc).await?;
                        return Ok(Some(path));
                    }
                    Err(e) => {
                        mark_failed(&key, e.ttl());
                        warn!(podcast_id, error = %e.error, permanent = e.permanent, "podcast art fetch failed");
                    }
                }
            }
        }
    } else {
        debug!(podcast_id, "podcast has no art_url");
    }

    if fallback_episode_art {
        // Most recently published episode of this podcast.
        let latest = episode::Entity::find()
            .filter(episode::Column::PodcastId.eq(podcast_id))
            .order_by_desc(episode::Column::PublishedAt)
            .one(dbc)
            .await?;
        if let Some(ep) = latest {
            warn!(
                podcast_id,
                episode_id = ep.id,
                "podcast art unavailable; falling back to latest episode art"
            );
            // Cross-call with the fallback OFF so the two methods can't loop.
            return Box::pin(ensure_episode_art(dbc, ep.id, media_root, false)).await;
        }
        debug!(podcast_id, "no episodes to borrow art from");
    }

    Ok(None)
}

/// Resolve a small (thumbnail) variant of an episode's artwork, generating it on first request. Resolves the
/// full-resolution original exactly like [`ensure_episode_art`] (same fetch/cache/fallback path), then lazily
/// derives a downscaled `<stem>.small.<ext>` sibling next to it. `Ok(None)` mirrors the original: no artwork
/// available.
pub async fn ensure_episode_art_small(
    dbc: &DatabaseConnection,
    episode_id: i32,
    media_root: &Path,
    fallback_podcast_art: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(original) =
        ensure_episode_art(dbc, episode_id, media_root, fallback_podcast_art).await?
    else {
        return Ok(None);
    };
    Ok(Some(ensure_small_variant(&original).await))
}

/// Small-variant counterpart to [`ensure_podcast_art`]. See [`ensure_episode_art_small`].
pub async fn ensure_podcast_art_small(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    media_root: &Path,
    fallback_episode_art: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(original) =
        ensure_podcast_art(dbc, podcast_id, media_root, fallback_episode_art).await?
    else {
        return Ok(None);
    };
    Ok(Some(ensure_small_variant(&original).await))
}

/// The small-variant path for an original art file: `<stem>.small.<ext>` next to
/// the original. `None` if the original has no usable stem/extension (an
/// `.img`-suffixed unknown type), in which case there's nothing to downscale to.
fn small_variant_path(original: &Path) -> Option<PathBuf> {
    let ext = original.extension()?.to_str()?;
    let stem = original.file_stem()?.to_str()?;
    // Convert PNG to lossless WebP to shrink thumbnails while preserving alpha. Keep other formats: lossless WebP can
    // enlarge JPEGs. The extension selects both encoder and served Content-Type.
    let out_ext = if ext.eq_ignore_ascii_case("png") {
        "webp"
    } else {
        ext
    };
    Some(original.with_file_name(format!("{stem}.small.{out_ext}")))
}

/// Ensure the downscaled sibling of `original` exists, returning its path. Any
/// failure (undecodable/unsupported format, encode error, IO) logs and falls
/// back to `original` — serving the full image is always better than serving a
/// broken one. Generation is one-shot: once written, later requests hit the file.
async fn ensure_small_variant(original: &Path) -> PathBuf {
    let Some(small) = small_variant_path(original) else {
        return original.to_path_buf();
    };
    if small.is_file() {
        return small;
    }
    // Bound concurrent CPU-bound generation (see [`SMALL_GEN_LIMIT`]). Re-check
    // after acquiring: under a list-scroll burst a concurrent request for the
    // same row may have generated the file while we waited, so we skip a
    // redundant decode/encode and just serve it.
    let _permit = SMALL_GEN_LIMIT.acquire().await.ok();
    if small.is_file() {
        return small;
    }
    match generate_small(original.to_path_buf(), small.clone()).await {
        Ok(()) => small,
        Err(e) => {
            warn!(original = %original.display(), error = %e, "small art generation failed; serving original");
            original.to_path_buf()
        }
    }
}

/// Decode `original`, downscale its longest edge to [`SMALL_MAX_DIM`] (preserving
/// aspect ratio; never upscales), and encode the thumbnail to `small` in the
/// original's format. CPU-bound, so it runs on a blocking thread. Writes through
/// a temp sibling + atomic rename so a concurrent reader never sees a partial file.
async fn generate_small(original: PathBuf, small: PathBuf) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let format = image::ImageFormat::from_path(&small)
            .map_err(|_| anyhow::anyhow!("unsupported art extension for {}", small.display()))?;
        let mut reader = image::ImageReader::open(&original)?.with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(SMALL_DECODE_MAX_ALLOC);
        reader.limits(limits);
        let img = reader.decode()?;
        // Downscale only — `thumbnail` would otherwise upscale a small original to
        // fill the box. An already-small image is re-encoded unchanged (still
        // written to the small path so the endpoint always has a file to serve).
        let thumb = if img.width() > SMALL_MAX_DIM || img.height() > SMALL_MAX_DIM {
            img.thumbnail(SMALL_MAX_DIM, SMALL_MAX_DIM)
        } else {
            img
        };
        let mut tmp = small.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        thumb.save_with_format(&tmp, format)?;
        std::fs::rename(&tmp, &small)?;
        Ok(())
    })
    .await?
}

/// A cached path that still exists on disk (a wiped media dir re-fetches).
fn existing(art_file_path: &Option<String>) -> Option<PathBuf> {
    let path = PathBuf::from(art_file_path.as_deref()?);
    path.is_file().then_some(path)
}

/// A failed art fetch, tagged by whether a retry could ever succeed. `permanent`
/// failures (non-image, SVG, 404, oversized) earn the long negative-cache TTL;
/// `transient` ones (network, 5xx, timeout, disk) earn the short one. `?` on a
/// `reqwest`/IO error defaults to transient.
struct ArtFetchError {
    permanent: bool,
    error: anyhow::Error,
}

impl ArtFetchError {
    fn transient(error: anyhow::Error) -> Self {
        Self {
            permanent: false,
            error,
        }
    }
    fn permanent(error: anyhow::Error) -> Self {
        Self {
            permanent: true,
            error,
        }
    }
    /// The negative-cache TTL this failure earns.
    fn ttl(&self) -> Duration {
        if self.permanent {
            PERMANENT_NEGATIVE_TTL
        } else {
            NEGATIVE_TTL
        }
    }
}

impl From<reqwest::Error> for ArtFetchError {
    fn from(e: reqwest::Error) -> Self {
        Self::transient(e.into())
    }
}

impl From<std::io::Error> for ArtFetchError {
    fn from(e: std::io::Error) -> Self {
        Self::transient(e.into())
    }
}

/// A 4xx (except 408/429) won't fix itself on retry; 5xx/408/429 might.
fn is_permanent_status(status: StatusCode) -> bool {
    status.is_client_error()
        && status != StatusCode::REQUEST_TIMEOUT
        && status != StatusCode::TOO_MANY_REQUESTS
}

/// Fetch artwork into `<media_root>/art/<stem>.<ext>` using response Content-Type; reject non-images, including
/// historical audio URLs. Use the memoized, short-timeout art client so dead CDNs cannot pin browser image handlers for
/// audio-sized timeouts.
async fn fetch_art(url: &str, media_root: &Path, stem: &str) -> Result<PathBuf, ArtFetchError> {
    debug!(url, stem, "fetching art_url");
    let mut response = art_client().get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        let err = anyhow::anyhow!("art fetch failed: {status} ({url})");
        return Err(if is_permanent_status(status) {
            ArtFetchError::permanent(err)
        } else {
            ArtFetchError::transient(err)
        });
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        .unwrap_or_default();
    if !content_type.starts_with("image/") {
        warn!(url, content_type, "refusing to cache non-image artwork");
        return Err(ArtFetchError::permanent(anyhow::anyhow!(
            "artwork at {url} is not an image ({content_type})"
        )));
    }
    // Reject SVG: a feed-controlled SVG can carry <script> that executes on a
    // direct same-origin navigation to the art endpoint (which is served outside
    // the SPA's CSP). The art cache stores raster formats only.
    if content_type == "image/svg+xml" {
        warn!(url, "refusing to cache SVG artwork");
        return Err(ArtFetchError::permanent(anyhow::anyhow!(
            "artwork at {url} is SVG (not cached)"
        )));
    }
    let ext = match content_type.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        _ => "img",
    };
    const MAX_ART_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
    // Reject early when the origin advertises an oversized body...
    if response
        .content_length()
        .is_some_and(|len| len > MAX_ART_BYTES as u64)
    {
        return Err(ArtFetchError::permanent(anyhow::anyhow!(
            "artwork at {url} is too large"
        )));
    }
    // ...but also enforce the cap while streaming: a chunked response omits
    // Content-Length, so `response.bytes()` would buffer it unbounded. `art_url`
    // is feed-controlled (an attacker-controlled origin), and the UI fetches art
    // per rendered row — an un-capped multi-GB body OOMs the server.
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_ART_BYTES {
            return Err(ArtFetchError::permanent(anyhow::anyhow!(
                "artwork at {url} exceeded {MAX_ART_BYTES} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }

    let dir = media_root.join("art");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{stem}.{ext}"));
    tokio::fs::write(&path, &bytes).await?;
    debug!(url, content_type, ext, bytes = bytes.len(), path = %path.display(), "cached art");
    Ok(path)
}

#[cfg(test)]
mod tests;
