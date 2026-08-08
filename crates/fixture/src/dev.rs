//! Development data seeding.
//!
//! [`seed_dev_data`] populates a fresh database with a rich, realistic dataset —
//! podcasts, episodes, podcast configs, subscriptions, playlists (including the
//! default queue), podcast→playlist auto-add links, and playback history — so a
//! local dev environment has every type of data the UI can render without
//! subscribing to anything by hand.
//!
//! Podcast + episode metadata is parsed straight from the RSS fixtures in
//! `data/tests/*.xml` (the same feeds the server's parser tests use). Every
//! downloaded episode points at a single copy of `data/tests/nasa-test-clip.mp3` placed in
//! the media root, so playback works offline without real downloads.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use rss::{Channel, Item};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter};
use tracing::{info, warn};

use halogen_orm::{
    episode, episode_chapter, episode_playlist, playback, playlist, podcast, podcast_auto_playlist,
    podcast_config, user, user_podcast,
};
use halogen_wire::DownloadStatus;

/// RSS fixtures to seed from, relative to the fixtures dir (`data/tests`). The
/// human-readable name comes from each feed's `<channel><title>`.
const FIXTURES: &[&str] = &[
    "hanselminutes.xml",
    "sed_podcast.xml",
    "npr_embedded.xml",
    "audioboom_no_such_thing.xml",
    "devotea.xml",
    // The Daily, pre-filtered to May 2026 episodes only (the full feed lives in
    // `simplecast_the_daily.xml`, which the RSS/polling tests use).
    "simplecast_the_daily_may.xml",
    // Odd Lots carries inline `psc:chapters` on a couple of episodes — seeded with
    // their chapter rows so the now-playing chapter UI has real data in dev (see
    // the chaptered-item handling in the seed loop). Acquired carries the external
    // `podcast:chapters` URL form, which can't be fetched offline, so it seeds as a
    // normal podcast (no chapter rows) — useful as the "URL but no chapters" case.
    "omny_odd_lots.xml",
    "transistor_acquired.xml",
];

/// Cap episodes ingested per feed — enough to populate lists without seeding
/// hundreds of rows per podcast.
const MAX_EPISODES_PER_PODCAST: usize = 10;

/// The single audio file every downloaded episode points at, placed in the media
/// root (a copy of `data/tests/nasa-test-clip.mp3`).
const SEED_AUDIO_FILENAME: &str = "seed_dev.mp3";

/// Seed a development dataset from the RSS fixtures in `fixtures_dir`
/// (typically `data/tests`), copying `nasa-test-clip.mp3` into `media_root` for playback.
///
/// Idempotent: if any podcast already exists, this is a no-op and returns
/// `Ok(false)`. On a fresh database it inserts the full dataset and returns
/// `Ok(true)`.
pub async fn seed_dev_data(
    dbc: &DatabaseConnection,
    fixtures_dir: &Path,
    media_root: &Path,
) -> Result<bool> {
    if podcast::Entity::find().count(dbc).await? > 0 {
        info!("Podcast(s) already exist — skipping dev data seed");
        return Ok(false);
    }

    // All dev data is owned by / attached to the first user (the seeded admin):
    // podcasts get `owner_id`, the user is subscribed to every podcast, playlists
    // are owned, playback history is attached. NOT NULL `owner_id` needs a user, so
    // skip the seed entirely if none exists.
    let owner_id = match user::Entity::find().one(dbc).await? {
        Some(u) => u.id,
        None => {
            warn!("No user present — skipping dev data seed");
            return Ok(false);
        }
    };

    // One shared audio file all downloaded episodes reference.
    let audio_path = stage_mock_audio(fixtures_dir, media_root).await?;

    let mut all_episodes: Vec<i32> = Vec::new();
    let mut downloaded_episodes: Vec<i32> = Vec::new();
    let mut podcast_ids: Vec<i32> = Vec::new();
    let mut chapters_seeded: usize = 0;

    for (idx, fixture) in FIXTURES.iter().enumerate() {
        let path = fixtures_dir.join(fixture);
        let xml = match std::fs::read_to_string(&path) {
            Ok(xml) => xml,
            Err(e) => {
                warn!("Skipping fixture {} (read failed): {e}", path.display());
                continue;
            }
        };
        let channel = match Channel::read_from(xml.as_bytes()) {
            Ok(channel) => channel,
            Err(e) => {
                warn!("Skipping fixture {fixture} (parse failed): {e}");
                continue;
            }
        };

        let config_id = insert_podcast_config(dbc, idx).await?;
        let podcast_id = insert_podcast(dbc, &channel, fixture, config_id, owner_id).await?;
        podcast_ids.push(podcast_id);

        // Seed the first N items, PLUS any later item that carries chapters, so the
        // chapter UI has real data even when a feed's chaptered episodes sit deep in
        // its back catalog (Odd Lots' are ~600+ items in). De-duped by index so a
        // chaptered item already in the first N isn't seeded twice.
        let items = channel.items();
        let mut to_seed: Vec<&Item> = items.iter().take(MAX_EPISODES_PER_PODCAST).collect();
        for item in items.iter().skip(MAX_EPISODES_PER_PODCAST) {
            if !item_chapters(item, fixtures_dir).is_empty() {
                to_seed.push(item);
            }
        }

        for (ep_idx, item) in to_seed.into_iter().enumerate() {
            // Leave every 5th episode un-downloaded so the NOT_DOWNLOADED state is
            // also represented in the seeded data.
            let downloaded = ep_idx % 5 != 0;
            let episode_id =
                insert_episode(dbc, podcast_id, item, ep_idx, &audio_path, downloaded).await?;
            all_episodes.push(episode_id);
            if downloaded {
                downloaded_episodes.push(episode_id);
            }
            // Chapters → chapter rows (read-only, like the sync path).
            let chapters = item_chapters(item, fixtures_dir);
            chapters_seeded += insert_chapters(dbc, episode_id, &chapters).await?;
        }
    }

    // Subscribe the user to every seeded podcast so their library lists them all.
    seed_subscriptions(dbc, owner_id, &podcast_ids).await?;

    let named_playlists = seed_playlists(dbc, &all_episodes, owner_id).await?;
    seed_podcast_auto_playlists(dbc, &podcast_ids, &named_playlists).await?;

    // Playback history is per-user; attach it to the seeded owner.
    seed_playbacks(dbc, owner_id, &downloaded_episodes).await?;

    info!(
        "Seeded dev data: {} podcasts, {} episodes ({} downloaded, {} chapters)",
        FIXTURES.len(),
        all_episodes.len(),
        downloaded_episodes.len(),
        chapters_seeded
    );
    Ok(true)
}

/// Copy the fixture `nasa-test-clip.mp3` into the media root, returning its absolute path
/// (the value stored in `episode.content_file_path`, matching the download
/// service's convention of an absolute path under the media root).
async fn stage_mock_audio(fixtures_dir: &Path, media_root: &Path) -> Result<String> {
    let src = fixtures_dir.join("nasa-test-clip.mp3");
    if !src.exists() {
        anyhow::bail!("mock audio fixture not found: {}", src.display());
    }
    tokio::fs::create_dir_all(media_root)
        .await
        .with_context(|| format!("create media root {}", media_root.display()))?;
    let dest: PathBuf = media_root.join(SEED_AUDIO_FILENAME);
    tokio::fs::copy(&src, &dest)
        .await
        .with_context(|| format!("copy {} -> {}", src.display(), dest.display()))?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Insert a podcast config with slightly varied values per podcast so the
/// config screen has distinct rows to show.
async fn insert_podcast_config(dbc: &DatabaseConnection, idx: usize) -> Result<i32> {
    let now = Utc::now();
    let model = podcast_config::ActiveModel {
        id: NotSet,
        poll_interval_seconds: Set(Some(3600 + (idx as u32) * 600)),
        max_episodes: Set(Some(50)),
        max_concurrent_downloads: Set(Some(3)),
        auto_download_enabled: Set(Some(false)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(podcast_config::Entity::insert(model)
        .exec(dbc)
        .await?
        .last_insert_id)
}

/// Insert a podcast from a parsed RSS channel.
async fn insert_podcast(
    dbc: &DatabaseConnection,
    channel: &Channel,
    fixture: &str,
    config_id: i32,
    owner_id: i32,
) -> Result<i32> {
    let now = Utc::now();

    let title = if channel.title().is_empty() {
        fixture.trim_end_matches(".xml").replace(['_', '-'], " ")
    } else {
        channel.title().to_string()
    };
    // We have no live feed URL for the fixtures; use the channel homepage when
    // present, otherwise a deterministic non-resolving placeholder.
    let feed_url = if channel.link().is_empty() {
        format!("https://podcasts.dev.local/{fixture}")
    } else {
        channel.link().to_string()
    };
    let art_url = channel
        .itunes_ext()
        .and_then(|ext| ext.image().map(str::to_string))
        .or_else(|| channel.image().map(|img| img.url().to_string()));
    let author = channel
        .itunes_ext()
        .and_then(|ext| ext.author().map(str::to_string))
        .or_else(|| channel.managing_editor().map(str::to_string));

    let model = podcast::ActiveModel {
        id: NotSet,
        title: Set(title),
        description: Set(channel.description().to_string()),
        feed_url: Set(feed_url),
        art_url: Set(art_url),
        author: Set(author),
        polled_at: Set(Some(now)),
        podcast_config_id: Set(Some(config_id)),
        owner_id: Set(owner_id),
        art_file_path: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        feed_url_redirects: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(podcast::Entity::insert(model)
        .exec(dbc)
        .await?
        .last_insert_id)
}

/// Insert an episode from a parsed RSS item. Downloaded episodes point at the
/// shared seed audio file and are marked `DOWNLOADED`.
async fn insert_episode(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    item: &Item,
    ep_idx: usize,
    audio_path: &str,
    downloaded: bool,
) -> Result<i32> {
    let now = Utc::now();

    let title = item
        .title()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Episode {}", ep_idx + 1));
    let description = item
        .description()
        .map(str::to_string)
        .or_else(|| item.content().map(str::to_string));
    let content_url = item
        .enclosure()
        .map(|enc| enc.url().to_string())
        .or_else(|| item.link().map(str::to_string))
        .unwrap_or_else(|| format!("https://podcasts.dev.local/audio/{podcast_id}-{ep_idx}.mp3"));
    let art_url = item
        .itunes_ext()
        .and_then(|ext| ext.image().map(str::to_string));
    let published_at = item
        .pub_date()
        .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| Some(now - Duration::days(ep_idx as i64)));
    let duration_secs = item
        .itunes_ext()
        .and_then(|ext| ext.duration())
        .and_then(parse_itunes_duration)
        .unwrap_or(1800 + (ep_idx as i32) * 120);

    let (download_status, downloaded_at, content_file_path) = if downloaded {
        (
            DownloadStatus::Downloaded,
            Some(now),
            Some(audio_path.to_string()),
        )
    } else {
        (DownloadStatus::NotDownloaded, None, None)
    };

    let model = episode::ActiveModel {
        id: NotSet,
        podcast_id: Set(podcast_id),
        title: Set(title),
        description: Set(description.unwrap_or_default()),
        content_url: Set(content_url),
        guid: Set(None),
        art_url: Set(art_url),
        published_at: Set(published_at),
        downloaded_at: Set(downloaded_at),
        content_file_path: Set(content_file_path),
        download_size: Set(None),
        art_file_path: Set(None),
        download_status: Set(download_status),
        download_started_at: Set(None),
        download_attempts: Set(0),
        duration_secs: Set(Some(duration_secs)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(episode::Entity::insert(model)
        .exec(dbc)
        .await?
        .last_insert_id)
}

/// Seed the default queue plus a couple of named playlists, spreading the
/// available episodes across them. Returns the ids of the named (non-default)
/// playlists, so callers can wire podcast auto-add links to them.
async fn seed_playlists(
    dbc: &DatabaseConnection,
    episodes: &[i32],
    user_id: i32,
) -> Result<Vec<i32>> {
    if episodes.is_empty() {
        warn!("No episodes seeded — skipping playlist seed");
        return Ok(Vec::new());
    }

    // The default queue (`is_default = true`) backs the /queue screen. Reuse the
    // one `seed_default_queue` made at startup rather than insert a second default
    // "Queue" (would trip the partial-unique is_default-per-user index). Create one
    // only if absent.
    let queue_id = match find_default_queue(dbc, user_id).await? {
        Some(id) => id,
        None => insert_playlist(dbc, "Queue", Some("Up next"), true, user_id).await?,
    };
    add_episodes_to_playlist(dbc, queue_id, episodes.iter().take(6).copied()).await?;

    let favorites_id =
        insert_playlist(dbc, "Favorites", Some("Episodes I love"), false, user_id).await?;
    add_episodes_to_playlist(dbc, favorites_id, episodes.iter().skip(6).take(5).copied()).await?;

    let later_id = insert_playlist(
        dbc,
        "Listen Later",
        Some("Saved for the commute"),
        false,
        user_id,
    )
    .await?;
    add_episodes_to_playlist(
        dbc,
        later_id,
        episodes.iter().skip(2).step_by(3).take(5).copied(),
    )
    .await?;

    Ok(vec![favorites_id, later_id])
}

/// Link a couple of podcasts to named playlists so the auto-playlist screen has
/// existing rows to show. Each linked podcast auto-adds new episodes to its
/// playlist when the RSS poller ingests them.
async fn seed_podcast_auto_playlists(
    dbc: &DatabaseConnection,
    podcast_ids: &[i32],
    playlist_ids: &[i32],
) -> Result<()> {
    if podcast_ids.is_empty() || playlist_ids.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    // Pair the first few podcasts with the named playlists round-robin.
    for (idx, &podcast_id) in podcast_ids.iter().take(playlist_ids.len()).enumerate() {
        let playlist_id = playlist_ids[idx % playlist_ids.len()];
        let model = podcast_auto_playlist::ActiveModel {
            podcast_id: Set(podcast_id),
            playlist_id: Set(playlist_id),
            add_to_start: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        podcast_auto_playlist::Entity::insert(model)
            .exec(dbc)
            .await?;
    }
    Ok(())
}

/// The existing default (`is_default`) queue's id, if one has already been
/// seeded (by `seed_default_queue` at startup).
async fn find_default_queue(dbc: &DatabaseConnection, user_id: i32) -> Result<Option<i32>> {
    Ok(playlist::Entity::find()
        .filter(playlist::Column::IsDefault.eq(true))
        .filter(playlist::Column::UserId.eq(user_id))
        .one(dbc)
        .await?
        .map(|p| p.id))
}

async fn insert_playlist(
    dbc: &DatabaseConnection,
    name: &str,
    description: Option<&str>,
    is_default: bool,
    user_id: i32,
) -> Result<i32> {
    let now = Utc::now();
    // Append to the end of the user's manual order: position = max + 1 (0 when
    // first). Mirrors the `playlist_store` handler.
    let position = playlist::Entity::find()
        .filter(playlist::Column::UserId.eq(user_id))
        .all(dbc)
        .await?
        .iter()
        .map(|p| p.position)
        .max()
        .map(|p| p + 1)
        .unwrap_or(0);
    let model = playlist::ActiveModel {
        id: NotSet,
        name: Set(name.to_string()),
        description: Set(description.map(str::to_string)),
        user_id: Set(user_id),
        is_default: Set(is_default),
        position: Set(position),
        on_remove_delete_file_server: Set(false),
        on_remove_delete_file_client: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
    };
    Ok(playlist::Entity::insert(model)
        .exec(dbc)
        .await?
        .last_insert_id)
}

async fn add_episodes_to_playlist(
    dbc: &DatabaseConnection,
    playlist_id: i32,
    episode_ids: impl Iterator<Item = i32>,
) -> Result<()> {
    let now = Utc::now();
    for (position, episode_id) in episode_ids.enumerate() {
        let model = episode_playlist::ActiveModel {
            episode_id: Set(episode_id),
            playlist_id: Set(playlist_id),
            position: Set(position as i32),
            created_at: Set(now),
            updated_at: Set(now),
        };
        episode_playlist::Entity::insert(model).exec(dbc).await?;
    }
    Ok(())
}

/// Subscribe a user to every podcast (a `user_podcast` row each), so the seeded
/// account's library lists all the seeded podcasts.
async fn seed_subscriptions(
    dbc: &DatabaseConnection,
    user_id: i32,
    podcast_ids: &[i32],
) -> Result<()> {
    let now = Utc::now();
    for &podcast_id in podcast_ids {
        let model = user_podcast::ActiveModel {
            user_id: Set(user_id),
            podcast_id: Set(podcast_id),
            created_at: Set(now),
            updated_at: Set(now),
        };
        user_podcast::Entity::insert(model).exec(dbc).await?;
    }
    Ok(())
}

/// Seed playback rows for a user: a mix of completed (history) and in-progress
/// (resumable) playbacks.
async fn seed_playbacks(dbc: &DatabaseConnection, user_id: i32, episodes: &[i32]) -> Result<()> {
    if episodes.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    for (i, &episode_id) in episodes.iter().take(8).enumerate() {
        let completed = i % 2 == 0;
        // Completed playbacks rest at the start; in-progress ones hold a cursor.
        let cursor = if completed { 0 } else { 300 + (i as i64) * 120 };
        let when = now - Duration::days(i as i64);
        let model = playback::ActiveModel {
            id: NotSet,
            user_id: Set(user_id),
            episode_id: Set(episode_id),
            cursor: Set(cursor),
            completed: Set(completed),
            created_at: Set(when),
            updated_at: Set(when),
        };
        playback::Entity::insert(model).exec(dbc).await?;
    }
    Ok(())
}

/// Seed a second, **non-admin** development account (`dev2` / password `dev2`) with
/// a small library: a default queue, one custom playlist, and three subscriptions —
/// two shared with the admin's seed plus one podcast only `dev2` follows. Lets the
/// dev UI exercise the multi-user / subscription-scoping paths the single admin
/// seed can't show.
///
/// Idempotent: a no-op returning `Ok(false)` if the `dev2` user already exists.
/// Best run after [`seed_dev_data`] so there are existing podcasts to share.
pub async fn seed_dev2_data(dbc: &DatabaseConnection) -> Result<bool> {
    if user::Entity::find()
        .filter(user::Column::Username.eq("dev2"))
        .one(dbc)
        .await?
        .is_some()
    {
        info!("dev2 user already exists — skipping dev2 seed");
        return Ok(false);
    }

    let now = Utc::now();

    // Non-admin user. Fixed id just below the admin's `i32::MAX` (mirroring the
    // admin seed's reserved-high id) so it never collides with auto-increment rows.
    let dev2_id = i32::MAX - 1;
    let password_hash = bcrypt::hash("dev2", bcrypt::DEFAULT_COST).context("hash dev2 password")?;
    user::Entity::insert(user::ActiveModel {
        id: Set(dev2_id),
        username: Set("dev2".to_string()),
        password_hash: Set(password_hash),
        is_admin: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(dbc)
    .await?;

    // Share up to two of the admin-seeded podcasts...
    let shared_ids: Vec<i32> = podcast::Entity::find()
        .all(dbc)
        .await?
        .into_iter()
        .take(2)
        .map(|p| p.id)
        .collect();

    // ...then one podcast only dev2 follows (owned by dev2, no episodes — it's here
    // purely as a subscription the admin doesn't have).
    let exclusive_id = podcast::Entity::insert(podcast::ActiveModel {
        id: NotSet,
        title: Set("Dev2 Exclusive".to_string()),
        description: Set("A podcast only the dev2 account is subscribed to.".to_string()),
        feed_url: Set("https://podcasts.dev.local/dev2-exclusive.xml".to_string()),
        art_url: Set(None),
        author: Set(Some("dev2".to_string())),
        polled_at: Set(Some(now)),
        podcast_config_id: Set(None),
        owner_id: Set(dev2_id),
        art_file_path: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        feed_url_redirects: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    })
    .exec(dbc)
    .await?
    .last_insert_id;

    let mut subscription_ids = shared_ids.clone();
    subscription_ids.push(exclusive_id);
    seed_subscriptions(dbc, dev2_id, &subscription_ids).await?;

    // Episodes from the shared podcasts populate dev2's playlists (the exclusive
    // podcast has none).
    let episode_ids: Vec<i32> = if shared_ids.is_empty() {
        Vec::new()
    } else {
        episode::Entity::find()
            .filter(episode::Column::PodcastId.is_in(shared_ids.clone()))
            .all(dbc)
            .await?
            .into_iter()
            .map(|e| e.id)
            .collect()
    };

    // Default queue + one custom playlist.
    let queue_id = insert_playlist(dbc, "Queue", Some("Up next"), true, dev2_id).await?;
    add_episodes_to_playlist(dbc, queue_id, episode_ids.iter().take(4).copied()).await?;

    let custom_id = insert_playlist(
        dbc,
        "Road Trip",
        Some("Saved for long drives"),
        false,
        dev2_id,
    )
    .await?;
    add_episodes_to_playlist(dbc, custom_id, episode_ids.iter().skip(4).take(4).copied()).await?;

    info!(
        "Seeded dev2 (non-admin) user: {} subscription(s) + queue + 1 custom playlist",
        subscription_ids.len()
    );
    Ok(true)
}

/// Dev-only: bundled local copies of external `podcast:chapters` JSON, keyed by
/// the URL in the fixture, so the offline seed can populate chapters that would
/// otherwise need a network fetch. Add an entry when a fixture's chaptered episode
/// uses the external URL form (the file lives alongside the XML in `data/tests`).
const BUNDLED_CHAPTER_JSON: &[(&str, &str)] = &[(
    "https://share.transistor.fm/s/95f9fcf7/chapters.json",
    "transistor_acquired_chapters.json",
)];

/// All chapters for a feed item, resolved OFFLINE as `(title, starts_at_secs)`:
/// inline `psc:chapters` when present, else a bundled local copy of the item's
/// external `podcast:chapters` JSON (dev substitute for the network fetch the real
/// sync path makes). Empty when the item has neither.
fn item_chapters(item: &Item, fixtures_dir: &Path) -> Vec<(String, i32)> {
    let inline = parse_psc_chapters(item);
    if !inline.is_empty() {
        return inline;
    }
    if let Some(url) = podcast_chapters_url(item)
        && let Some((_, file)) = BUNDLED_CHAPTER_JSON.iter().find(|(u, _)| *u == url)
    {
        return parse_chapters_json(&fixtures_dir.join(file)).unwrap_or_else(|e| {
            warn!("Failed to read bundled chapters {file}: {e}");
            Vec::new()
        });
    }
    Vec::new()
}

/// Insert pre-resolved chapter rows for an episode. Returns the number inserted.
async fn insert_chapters(
    dbc: &DatabaseConnection,
    episode_id: i32,
    chapters: &[(String, i32)],
) -> Result<usize> {
    if chapters.is_empty() {
        return Ok(0);
    }
    let now = Utc::now();
    for (title, starts_at_secs) in chapters {
        let model = episode_chapter::ActiveModel {
            id: NotSet,
            episode_id: Set(episode_id),
            title: Set(title.clone()),
            starts_at_secs: Set(*starts_at_secs),
            created_at: Set(now),
            updated_at: Set(now),
        };
        episode_chapter::Entity::insert(model).exec(dbc).await?;
    }
    Ok(chapters.len())
}

/// Parse inline `psc:chapters` off a feed item into `(title, starts_at_secs)`
/// pairs. `rss` exposes namespaced elements via `extensions()` keyed by prefix →
/// local name: `<psc:chapters>` at `["psc"]["chapters"]`, each `<psc:chapter>` a
/// child carrying `start` (NPT) + `title` attrs. Mirrors the server's parser; kept
/// here because the seed lives below the server crate.
fn parse_psc_chapters(item: &Item) -> Vec<(String, i32)> {
    let Some(psc) = item.extensions().get("psc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for wrapper in psc.values().flatten() {
        for chapter in wrapper.children().values().flatten() {
            let attrs = chapter.attrs();
            if let (Some(title), Some(start)) = (attrs.get("title"), attrs.get("start"))
                && let Some(secs) = parse_chapter_start(start)
            {
                out.push((title.clone(), secs));
            }
        }
    }
    out
}

/// The external `podcast:chapters` URL on a feed item, if any. `rss` exposes it at
/// `extensions()["podcast"]["chapters"]` with the URL in the `url` attr.
fn podcast_chapters_url(item: &Item) -> Option<String> {
    item.extensions()
        .get("podcast")?
        .get("chapters")?
        .iter()
        .find_map(|ext| ext.attrs().get("url").cloned())
}

/// Parse a Podcasting 2.0 chapters JSON document into `(title, starts_at_secs)`
/// pairs: the top-level `chapters` array's `title` + `startTime` (truncated to
/// whole seconds). Trims titles (feeds often pad them with `\r`) and skips entries
/// missing either field or with a negative/non-finite start.
fn parse_chapters_json(path: &Path) -> Result<Vec<(String, i32)>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read chapters json {}", path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&raw)?;
    let out = doc
        .get("chapters")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|ch| {
                    let title = ch.get("title").and_then(|t| t.as_str())?.trim().to_string();
                    let start = ch.get("startTime").and_then(|s| s.as_f64())?;
                    if title.is_empty() || !start.is_finite() || start < 0.0 {
                        return None;
                    }
                    Some((title, start as i32))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(out)
}

/// Parse a `psc:chapter` `start` (NPT `HH:MM:SS(.mmm)`) into whole seconds,
/// dropping any fractional-seconds suffix before reusing the duration parser.
fn parse_chapter_start(raw: &str) -> Option<i32> {
    let whole = raw.trim().split('.').next().unwrap_or(raw);
    parse_itunes_duration(whole)
}

/// Parse an iTunes `<itunes:duration>` value into seconds. Accepts plain seconds
/// (`"2814"`), `MM:SS` (`"46:54"`), or `HH:MM:SS` (`"1:02:03"`).
fn parse_itunes_duration(raw: &str) -> Option<i32> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(secs) = raw.parse::<i32>() {
        return Some(secs);
    }
    let parts: Option<Vec<i32>> = raw
        .split(':')
        .map(|p| p.trim().parse::<i32>().ok())
        .collect();
    match parts?.as_slice() {
        [h, m, s] => Some(h * 3600 + m * 60 + s),
        [m, s] => Some(m * 60 + s),
        [s] => Some(*s),
        _ => None,
    }
}
