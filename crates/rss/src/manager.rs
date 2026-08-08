//! The RSS sync orchestrator.
//!
//! [`RssManager`] owns the per-run dependencies (DB handle, HTTP client, resolved
//! [`SyncContext`]) so the orchestration methods don't thread them around. The
//! free functions at the bottom are thin entry points that build a manager and run
//! it — the poller and tests call those.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;

use chrono::{DateTime, Utc};
use halogen_orm::episode::{
    ActiveModel as EpisodeActiveModel, Column, Entity as EpisodeEntity, Model as EpisodeModel,
};
use halogen_orm::episode_chapter::{
    ActiveModel as EpisodeChapterActiveModel, Entity as EpisodeChapterEntity,
};
use halogen_orm::podcast::{ActiveModel as PodcastActiveModel, Entity as PodcastEntity, Model};
use halogen_orm::podcast_config::{Entity as PodcastConfigEntity, Model as PodcastConfigModel};
use halogen_utils::constants::MAX_FEED_BODY_BYTES;
use halogen_wire::PodcastPollResultData;
use reqwest::Client;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect, Set, TransactionTrait,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{info, warn};

use super::feed::parse_feed;
use super::types::{PodcastSyncOutcome, RemoteChapter, RemoteEpisodeData, SyncContext};
use halogen_download as download;
use halogen_wire::outcome_for;

/// `Set` the value when the response carried the header, else `NotSet` so the
/// stored conditional-GET validator is PRESERVED — a `304` (or a `200` that omits
/// `ETag`/`Last-Modified`) must not wipe it, or the next poll can't revalidate and
/// re-fetches the whole feed.
fn set_if_present(v: Option<String>) -> sea_orm::ActiveValue<Option<String>> {
    match v {
        Some(x) => Set(Some(x)),
        None => sea_orm::ActiveValue::NotSet,
    }
}

/// Per-podcast knobs resolved from `podcast_config` over the context fallbacks.
struct Resolved {
    poll_interval: Duration,
    keep: usize,
    max_dl: usize,
    auto: bool,
}

/// Drives a podcast-feed sync run with a shared DB handle, HTTP client, and the
/// resolved [`SyncContext`]. Cloned per podcast so each feed syncs on its own task.
#[derive(Clone)]
pub struct RssManager {
    dbc: DatabaseConnection,
    /// Client for polling feeds: redirects are NOT auto-followed so we can record
    /// the hop chain by hand (see [`Self::fetch_feed_following_redirects`]).
    /// Episode downloads build their own auto-following client in `download`.
    feed_client: Client,
    ctx: SyncContext,
}

impl RssManager {
    pub fn new(dbc: DatabaseConnection, ctx: SyncContext) -> Self {
        Self {
            dbc,
            feed_client: download::feed_client(),
            ctx,
        }
    }

    /// Sync all podcasts (or just `podcast_ids`), reporting each podcast's outcome
    /// to `on_result` as its task finishes. Honours per-podcast interval gating,
    /// auto-download, and retention per the [`SyncContext`].
    pub async fn run<F>(&self, podcast_ids: Option<Vec<i32>>, on_result: F) -> Result<(), String>
    where
        F: Fn(PodcastPollResultData),
    {
        let mut query = PodcastEntity::find();
        if let Some(ids) = &podcast_ids {
            query = query.filter(halogen_orm::podcast::Column::Id.is_in(ids.iter().copied()));
        }
        let podcasts = query
            .all(&self.dbc)
            .await
            .map_err(|e| format!("Failed to fetch podcasts: {e}"))?;

        if podcasts.is_empty() {
            info!("No podcasts found, skipping sync");
            return Ok(());
        }

        let configs = self.load_configs(&podcasts).await?;
        let now = Utc::now();
        let semaphore = Arc::new(Semaphore::new(self.ctx.max_poll_concurrent.max(1)));
        let mut total_new = 0usize;
        let mut total_updated = 0usize;
        let mut total_errors = 0usize;
        // Each task carries its own (id, title) so results can be reported in
        // COMPLETION order via `JoinSet` — a slow feed no longer blocks reporting
        // of faster ones behind it. `sync_one` is wrapped in `catch_unwind` so a
        // panic (or a hard `Err`) becomes that podcast's error outcome instead of
        // aborting the whole run and dropping every other podcast's result.
        let mut set: JoinSet<(i32, String, Result<PodcastSyncOutcome, String>)> = JoinSet::new();

        for podcast in podcasts {
            let resolved = self.resolve(&podcast, &configs);

            // Interval gating (scheduled runs only). A never-polled podcast or a
            // clock skew (negative elapsed) always polls.
            if self.ctx.respect_poll_interval && not_due(now, &podcast, resolved.poll_interval) {
                continue;
            }

            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| format!("Failed to acquire semaphore permit: {e}"))?;
            let mgr = self.clone();
            let podcast_id = podcast.id;
            let title = podcast.title.clone();

            set.spawn(async move {
                let _permit = permit;
                let result = AssertUnwindSafe(mgr.sync_one(podcast, resolved))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err("Polling task panicked".to_string()));
                (podcast_id, title, result)
            });
        }

        while let Some(joined) = set.join_next().await {
            let (podcast_id, title, result) = match joined {
                Ok(tuple) => tuple,
                // The task body maps panics to `Err` above, so a `JoinError` here
                // means the task was cancelled — log and keep draining the rest.
                Err(e) => {
                    warn!("Polling task join error: {e}");
                    continue;
                }
            };
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(e) => {
                    // Hard failure (task panic or an unrecoverable Err) — the
                    // per-step failure points inside `sync_podcast` record their
                    // own reasons; this catches everything they can't.
                    warn!("Polling podcast {podcast_id} failed: {e}");
                    self.record_sync_error(podcast_id, format!("Sync failed: {e}"))
                        .await;
                    errored_outcome()
                }
            };
            total_new += outcome.new;
            total_updated += outcome.updated;
            total_errors += outcome.errors;
            on_result(PodcastPollResultData {
                podcast_id,
                title,
                outcome: outcome_for(
                    outcome.new,
                    outcome.updated,
                    outcome.errors,
                    outcome.skipped,
                ),
                new_episodes: outcome.new,
                updated_episodes: outcome.updated,
                errors: outcome.errors,
            });
        }

        info!(
            "Sync completed: {} new, {} updated, {} errors",
            total_new, total_updated, total_errors
        );
        Ok(())
    }

    /// Batch-load the linked `podcast_config` rows so per-podcast resolution is a
    /// map lookup rather than a query per feed.
    async fn load_configs(
        &self,
        podcasts: &[Model],
    ) -> Result<HashMap<i32, PodcastConfigModel>, String> {
        let config_ids: Vec<i32> = podcasts
            .iter()
            .filter_map(|p| p.podcast_config_id)
            .collect();
        if config_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let map = PodcastConfigEntity::find()
            .filter(halogen_orm::podcast_config::Column::Id.is_in(config_ids))
            .all(&self.dbc)
            .await
            .map_err(|e| format!("Failed to fetch podcast configs: {e}"))?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        Ok(map)
    }

    /// Resolve a podcast's knobs: its `podcast_config` override, else the context
    /// fallback.
    fn resolve(&self, podcast: &Model, configs: &HashMap<i32, PodcastConfigModel>) -> Resolved {
        let config = podcast.podcast_config_id.and_then(|id| configs.get(&id));
        Resolved {
            poll_interval: config
                .and_then(|c| c.poll_interval_seconds)
                .map(|s| Duration::from_secs(s as u64))
                .unwrap_or(self.ctx.fallback_poll_interval),
            keep: config
                .and_then(|c| c.max_episodes)
                .map(|v| v as usize)
                .unwrap_or(self.ctx.fallback_max_episodes),
            max_dl: config
                .and_then(|c| c.max_concurrent_downloads)
                .map(|v| v as usize)
                .unwrap_or(self.ctx.max_concurrent_downloads)
                .max(1),
            auto: config
                .and_then(|c| c.auto_download_enabled)
                .unwrap_or(self.ctx.auto_download_enabled),
        }
    }

    /// Sync one podcast, then (when auto-download is on) fetch its new episodes and
    /// enforce the retention cap.
    async fn sync_one(&self, podcast: Model, r: Resolved) -> Result<PodcastSyncOutcome, String> {
        let podcast_id = podcast.id;
        let outcome = self.sync_podcast(&podcast).await?;
        if r.auto {
            if !outcome.new_ids.is_empty() {
                self.auto_download_new(&outcome.new_ids, r.keep, r.max_dl)
                    .await;
            }
            if let Err(e) = download::enforce_retention(&self.dbc, podcast_id, r.keep).await {
                warn!("Retention failed for podcast {podcast_id}: {e}");
            }
        }
        Ok(outcome)
    }

    /// Download the just-ingested episodes oldest published first (so `downloaded_at`
    /// order tracks publish order — retention purges by `downloaded_at`). Only the
    /// newest `keep` are worth fetching (`keep == 0` = no cap, matching
    /// `enforce_retention`); at most `max_concurrent` run at once.
    async fn auto_download_new(&self, new_ids: &[i32], keep: usize, max_concurrent: usize) {
        if new_ids.is_empty() {
            return;
        }

        let mut eps = match EpisodeEntity::find()
            .filter(Column::Id.is_in(new_ids.iter().copied()))
            .all(&self.dbc)
            .await
        {
            Ok(eps) => eps,
            Err(e) => {
                warn!("auto-download: failed to load new episodes: {e}");
                return;
            }
        };
        // Oldest first; drop any beyond the newest `keep` (retention would purge
        // them). `keep == 0` means "no cap" (like `enforce_retention`) → fetch all.
        eps.sort_by(|a, b| a.published_at.cmp(&b.published_at).then(a.id.cmp(&b.id)));
        if keep != 0 && eps.len() > keep {
            eps.drain(0..eps.len() - keep);
        }

        let ids: Vec<i32> = eps.into_iter().map(|e| e.id).collect();
        download::download_many(&self.dbc, ids, &self.ctx.download_options(), max_concurrent).await;
    }

    /// GET `feed_url`, following redirects by hand (the feed client has auto-follow
    /// disabled) so the full hop chain can be recorded. Returns the final non-3xx
    /// response together with the chain `[feed_url, hop1, .., final]`; a direct feed
    /// yields just `[feed_url]`.
    ///
    /// Conditional headers are re-sent on each hop. gzip is handled transparently by
    /// reqwest's `gzip` feature, so we never set `Accept-Encoding` by hand (that
    /// leaves the body compressed). An empty etag / last-modified is skipped — an
    /// empty `If-Modified-Since` is an invalid HTTP-date some origins reject. Note a
    /// `304 Not Modified` carries no `Location`, so it falls through and is returned
    /// as the final response even though it sits in the 3xx range.
    async fn fetch_feed_following_redirects(
        &self,
        feed_url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<(reqwest::Response, Vec<String>), reqwest::Error> {
        // Mirror reqwest's default cap; also breaks redirect loops.
        const MAX_HOPS: usize = 10;
        let mut chain = vec![feed_url.to_string()];
        let mut current = feed_url.to_string();
        let mut visited: HashSet<String> = HashSet::from([current.clone()]);
        loop {
            let mut request = self.feed_client.get(&current);
            if let Some(etag) = etag.filter(|s| !s.is_empty()) {
                request = request.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = last_modified.filter(|s| !s.is_empty()) {
                request = request.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
            }
            // SSRF: the feed client's DNS resolver rejects non-public hosts on every
            // hop (incl. these manual redirects), so no per-hop check is needed here.
            let resp = request.send().await?;

            if resp.status().is_redirection() && chain.len() <= MAX_HOPS {
                // Resolve the (possibly relative) Location against the current URL.
                let next = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|loc| reqwest::Url::parse(&current).ok()?.join(loc).ok());
                if let Some(next) = next {
                    let next = next.to_string();
                    // Stop on a redirect cycle instead of burning the whole hop budget.
                    if !visited.insert(next.clone()) {
                        return Ok((resp, chain));
                    }
                    chain.push(next.clone());
                    current = next;
                    continue;
                }
            }
            return Ok((resp, chain));
        }
    }

    /// Persist one sync-failure reason for `podcast_id` — the durable trail
    /// behind the admin Server Errors page (capped per podcast; see
    /// `podcast_sync_error::MAX_PER_PODCAST`). Best-effort: a failed write is
    /// only logged — the outcome's error count already reports the failure.
    async fn record_sync_error(&self, podcast_id: i32, reason: String) {
        if let Err(e) =
            halogen_orm::podcast_sync_error::Entity::record(&self.dbc, podcast_id, reason).await
        {
            warn!("Failed to persist sync error for podcast {podcast_id}: {e}");
        }
    }

    /// Fetch + ingest one podcast's feed: conditional GET (manual redirect
    /// following), 304 short-circuit, cutoff filter, then dedupe-by-guid/url and
    /// insert/heal each remote episode. Orchestration only — each step is a helper
    /// below, so the flow is a flat sequence of guards.
    async fn sync_podcast(&self, podcast: &Model) -> Result<PodcastSyncOutcome, String> {
        let dbc = &self.dbc;
        info!("Syncing podcast: {} ({})", podcast.title, podcast.feed_url);

        // Conditional GET, following redirects by hand so the full hop chain is
        // captured (the feed client has auto-follow disabled).
        let (mut resp, redirect_chain) = match self
            .fetch_feed_following_redirects(
                &podcast.feed_url,
                podcast.etag.as_deref(),
                podcast.last_modified.as_deref(),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to fetch feed for '{}': {}", podcast.title, e);
                self.record_sync_error(podcast.id, format!("Failed to fetch feed: {e}"))
                    .await;
                return Ok(errored_outcome());
            }
        };

        // CSV of every hop, starting with `feed_url`; a direct feed equals
        // `feed_url` exactly, which is how the UI detects "no redirect".
        let redirects_csv = redirect_chain.join(",");
        let (etag, last_modified) = extract_validators(&resp);

        // 304: stamp polled_at + validators and stop (no body to parse).
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            info!("Podcast '{}' feed not modified, skipping", podcast.title);
            self.stamp_polled(podcast, etag, last_modified, redirects_csv, None, None)
                .await?;
            return Ok(skipped_outcome());
        }

        // Read + parse the body; either failure is a per-podcast error, not fatal.
        // The body is capped at `MAX_FEED_BODY_BYTES` so a hostile/misconfigured
        // feed host can't exhaust memory with a multi-gigabyte response.
        let body = match read_feed_body(&mut resp).await {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    "Failed to read response body for '{}': {}",
                    podcast.title, e
                );
                self.record_sync_error(podcast.id, format!("Failed to read feed body: {e}"))
                    .await;
                return Ok(errored_outcome());
            }
        };
        let feed = match parse_feed(&body) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to parse RSS feed for '{}': {}", podcast.title, e);
                self.record_sync_error(podcast.id, format!("Failed to parse RSS feed: {e}"))
                    .await;
                return Ok(errored_outcome());
            }
        };

        let channel_art = feed.art_url;
        let channel_title = feed.channel_title;
        let remote_episodes = self.filter_by_cutoff(feed.episodes);

        // Stamp polled_at (and adopt changed channel art/title). Non-fatal on failure.
        let mut errors = 0usize;
        if let Err(e) = self
            .stamp_polled(
                podcast,
                etag,
                last_modified,
                redirects_csv,
                channel_art,
                channel_title,
            )
            .await
        {
            warn!("{e}");
            errors += 1;
        } else {
            info!("Updated polled_at for '{}'", podcast.title);
        }

        // Nothing survived the cutoff — skip the existing-episode query + insert loop.
        if remote_episodes.is_empty() {
            return Ok(PodcastSyncOutcome {
                new: 0,
                updated: 0,
                errors,
                skipped: false,
                new_ids: Vec::new(),
            });
        }

        // Load the existing set for dedup; without it we can't safely insert.
        let existing = match EpisodeEntity::find()
            .filter(Column::PodcastId.eq(podcast.id))
            .all(dbc)
            .await
        {
            Ok(existing) => existing,
            Err(e) => {
                warn!("Failed to fetch episodes for '{}': {}", podcast.title, e);
                self.record_sync_error(
                    podcast.id,
                    format!("Failed to load existing episodes for dedup: {e}"),
                )
                .await;
                return Ok(PodcastSyncOutcome {
                    new: 0,
                    updated: 0,
                    errors: errors + 1,
                    skipped: false,
                    new_ids: Vec::new(),
                });
            }
        };

        let ingested = self
            .ingest_episodes(podcast, remote_episodes, &existing)
            .await;

        // Auto-add freshly-ingested episodes to any playlists this podcast feeds.
        // Best-effort: a failure here must never fail the poll.
        if !ingested.new_ids.is_empty()
            && let Err(e) = auto_add_to_playlists(
                dbc,
                podcast.id,
                &ingested.new_ids,
                self.ctx.auto_playlist_add_to_start,
            )
            .await
        {
            warn!(
                "Failed to auto-add new episodes of '{}' to playlists: {}",
                podcast.title, e
            );
        }

        Ok(PodcastSyncOutcome {
            new: ingested.new,
            updated: ingested.updated,
            errors: errors + ingested.errors,
            skipped: false,
            new_ids: ingested.new_ids,
        })
    }

    /// Drop episodes published before the configured cutoff. Episodes with no pub
    /// date are kept (we can't prove they're too old). Runs before any DB work so a
    /// large back catalog is never queried-for or inserted.
    fn filter_by_cutoff(&self, episodes: Vec<RemoteEpisodeData>) -> Vec<RemoteEpisodeData> {
        match self.ctx.no_sync_before {
            Some(cutoff) => episodes
                .into_iter()
                .filter(|r| r.published_at.map(|p| p >= cutoff).unwrap_or(true))
                .collect(),
            None => episodes,
        }
    }

    /// Stamp `polled_at` + the conditional-GET validators (and adopt changed
    /// channel art) onto the podcast row. `channel_art` is `None` on the 304 path.
    /// Never clears working art with `None` — a transiently broken feed shouldn't
    /// erase it; the cached file is dropped on a real change (re-fetched lazily).
    async fn stamp_polled(
        &self,
        podcast: &Model,
        etag: Option<String>,
        last_modified: Option<String>,
        redirects_csv: String,
        channel_art: Option<String>,
        channel_title: Option<String>,
    ) -> Result<(), String> {
        let mut update = PodcastActiveModel {
            id: Set(podcast.id),
            polled_at: Set(Some(Utc::now())),
            etag: set_if_present(etag),
            last_modified: set_if_present(last_modified),
            feed_url_redirects: Set(Some(redirects_csv)),
            ..Default::default()
        };
        if channel_art.is_some() && channel_art != podcast.art_url {
            update.art_url = Set(channel_art);
            update.art_file_path = Set(None);
        }
        // Heal a placeholder title (subscribe-by-URL stores the feed URL as the
        // title) with the real channel title. Never touches a user-chosen title.
        if let Some(t) = channel_title
            && (podcast.title.is_empty() || podcast.title == podcast.feed_url)
        {
            update.title = Set(t.chars().take(256).collect());
        }
        PodcastEntity::update(update)
            .exec(&self.dbc)
            .await
            .map_err(|e| format!("Failed to update polled_at for '{}': {e}", podcast.title))?;
        Ok(())
    }

    /// Dedupe `remote_episodes` against `existing` (by stable `<guid>`, else
    /// enclosure URL) and apply each: heal a few legacy fields on a known episode,
    /// or insert a new one. A single identity map resolves match-or-insert in one
    /// lookup.
    async fn ingest_episodes(
        &self,
        podcast: &Model,
        remote_episodes: Vec<RemoteEpisodeData>,
        existing: &[EpisodeModel],
    ) -> Ingested {
        let dbc = &self.dbc;
        // Episode identity: stable RSS <guid> when present, else the enclosure URL.
        // URLs often carry rotating tracking params, so guid avoids re-inserting
        // the same episode.
        let by_guid: HashMap<&str, &EpisodeModel> = existing
            .iter()
            .filter_map(|ep| ep.guid.as_deref().map(|g| (g, ep)))
            .collect();
        let by_url: HashMap<&str, &EpisodeModel> = existing
            .iter()
            .map(|ep| (ep.content_url.as_str(), ep))
            .collect();

        let mut out = Ingested::default();
        for mut remote in remote_episodes {
            let matched = remote
                .guid
                .as_deref()
                .and_then(|g| by_guid.get(g).copied())
                .or_else(|| by_url.get(remote.content_url.as_str()).copied());

            match matched {
                Some(ep) => {
                    if let Some(model) = heal_episode(ep, &remote) {
                        match EpisodeEntity::update(model).exec(dbc).await {
                            Ok(_) => {
                                out.updated += 1;
                                info!("Updated episode: {}", remote.title);
                            }
                            Err(e) => {
                                warn!("Failed to update episode '{}': {}", remote.title, e);
                                out.errors += 1;
                            }
                        }
                    }
                }
                None => {
                    let title = remote.title.clone();
                    // Take the parsed chapters out before `new_episode` consumes the
                    // rest; they're persisted to their own table once we have an id.
                    let inline_chapters = std::mem::take(&mut remote.chapters);
                    let chapters_url = remote.chapters_url.take();
                    match EpisodeEntity::insert(new_episode(podcast.id, remote))
                        .exec(dbc)
                        .await
                    {
                        Ok(res) => {
                            let episode_id = res.last_insert_id;
                            out.new += 1;
                            out.new_ids.push(episode_id);
                            info!("Added new episode: {}", title);
                            // Best-effort: chapters are optional and must never fail
                            // or stall the sync. The external `podcast:chapters`
                            // fetch is suppressed under `use_mock_download` (tests /
                            // offline) so a fixture sync never hits the network;
                            // inline `psc` chapters still persist.
                            persist_chapters(
                                dbc,
                                episode_id,
                                inline_chapters,
                                chapters_url,
                                !self.ctx.use_mock_download,
                            )
                            .await;
                        }
                        Err(e) => {
                            warn!("Failed to insert episode '{}': {}", title, e);
                            out.errors += 1;
                        }
                    }
                }
            }
        }
        out
    }
}

/// Tallies from one `ingest_episodes` pass.
#[derive(Default)]
struct Ingested {
    new: usize,
    updated: usize,
    errors: usize,
    new_ids: Vec<i32>,
}

/// A per-podcast outcome carrying a single error (fetch/read/parse failures).
fn errored_outcome() -> PodcastSyncOutcome {
    PodcastSyncOutcome {
        new: 0,
        updated: 0,
        errors: 1,
        skipped: false,
        new_ids: Vec::new(),
    }
}

/// A per-podcast outcome for the 304-Not-Modified case (feed unchanged).
fn skipped_outcome() -> PodcastSyncOutcome {
    PodcastSyncOutcome {
        new: 0,
        updated: 0,
        errors: 0,
        skipped: true,
        new_ids: Vec::new(),
    }
}

/// The `ETag` + `Last-Modified` conditional-GET validators from a response.
fn extract_validators(resp: &reqwest::Response) -> (Option<String>, Option<String>) {
    let headers = resp.headers();
    let get = |name: reqwest::header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    (
        get(reqwest::header::ETAG),
        get(reqwest::header::LAST_MODIFIED),
    )
}

/// Heal a few legacy fields on a known episode from its remote counterpart,
/// returning the `ActiveModel` to persist when something changed (else `None`):
/// backfill `published_at`/`duration_secs` and adopt a changed `art_url` (dropping
/// the cached file, which belonged to the old URL).
fn heal_episode(ep: &EpisodeModel, remote: &RemoteEpisodeData) -> Option<EpisodeActiveModel> {
    let mut model = EpisodeActiveModel::from(ep.clone());
    let mut changed = false;
    if let Some(published) = remote.published_at
        && ep.published_at != Some(published)
    {
        model.published_at = Set(Some(published));
        changed = true;
    }
    if ep.duration_secs.is_none() && remote.duration_secs.is_some() {
        model.duration_secs = Set(remote.duration_secs);
        changed = true;
    }
    if ep.art_url != remote.art_url {
        model.art_url = Set(remote.art_url.clone());
        model.art_file_path = Set(None);
        changed = true;
    }
    changed.then_some(model)
}

/// Build the `ActiveModel` for a brand-new episode under `podcast_id`.
fn new_episode(podcast_id: i32, remote: RemoteEpisodeData) -> EpisodeActiveModel {
    let now = Utc::now();
    EpisodeActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        podcast_id: Set(podcast_id),
        title: Set(remote.title),
        // Column is NOT NULL; feeds without a <description> store an empty string
        // rather than 500 on insert.
        description: Set(remote.description.unwrap_or_default()),
        content_url: Set(remote.content_url),
        guid: Set(remote.guid),
        art_url: Set(remote.art_url),
        published_at: Set(remote.published_at),
        downloaded_at: Set(None),
        content_file_path: Set(None),
        download_size: Set(None),
        art_file_path: Set(None),
        download_status: Set(halogen_wire::DownloadStatus::NotDownloaded),
        download_started_at: Set(None),
        download_attempts: Set(0),
        duration_secs: Set(remote.duration_secs),
        created_at: Set(now),
        updated_at: Set(now),
    }
}

/// Persist a freshly-inserted episode's chapters, best-effort. Inline `psc`
/// chapters are stored directly; otherwise a `podcast:chapters` URL is fetched and
/// parsed. ANY failure (network, oversized/garbage JSON, DB error) is logged and
/// swallowed — chapters are optional and must never fail or slow a feed sync.
/// No-op when the episode carries neither inline chapters nor a URL.
async fn persist_chapters(
    dbc: &DatabaseConnection,
    episode_id: i32,
    inline: Vec<RemoteChapter>,
    chapters_url: Option<String>,
    allow_remote_fetch: bool,
) {
    let chapters = if !inline.is_empty() {
        inline
    } else if let Some(url) = chapters_url.filter(|_| allow_remote_fetch) {
        match fetch_remote_chapters(&url).await {
            Ok(chapters) => chapters,
            Err(e) => {
                warn!("Skipping chapters for episode {episode_id} ({url}): {e}");
                return;
            }
        }
    } else {
        return;
    };
    if chapters.is_empty() {
        return;
    }
    let now = Utc::now();
    let models: Vec<EpisodeChapterActiveModel> = chapters
        .into_iter()
        .map(|c| EpisodeChapterActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            episode_id: Set(episode_id),
            title: Set(c.title),
            starts_at_secs: Set(c.starts_at_secs),
            created_at: Set(now),
            updated_at: Set(now),
        })
        .collect();
    if let Err(e) = EpisodeChapterEntity::insert_many(models).exec(dbc).await {
        warn!("Failed to store chapters for episode {episode_id}: {e}");
    }
}

/// Read a feed response body, capped at [`MAX_FEED_BODY_BYTES`]. The body comes
/// from a (user-supplied) feed URL and is buffered whole for parsing, so an
/// unbounded read is a memory-exhaustion vector — reject an oversized response
/// (advertised or streamed) rather than buffering it. Decoded lossily as UTF-8,
/// which the parser (`Channel::from_str`) requires; non-UTF-8 feeds are rare and
/// were already forced through a `String` by the previous `resp.text()`.
async fn read_feed_body(resp: &mut reqwest::Response) -> anyhow::Result<String> {
    // Reject early when the server advertises an oversized body.
    if let Some(len) = resp.content_length()
        && len > MAX_FEED_BODY_BYTES
    {
        anyhow::bail!("feed body too large (advertised {len} bytes)");
    }

    // Stream with the cap enforced as we go (chunked responses omit Content-Length).
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() as u64 + chunk.len() as u64 > MAX_FEED_BODY_BYTES {
            anyhow::bail!("feed body exceeded {MAX_FEED_BODY_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Largest `podcast:chapters` document we'll buffer. Real chapter files are a few
/// KB; the cap stops a hostile/misconfigured host from ballooning memory.
const MAX_CHAPTERS_BYTES: usize = 2 * 1024 * 1024;

/// Fetch + parse a Podcasting 2.0 `podcast:chapters` JSON document into markers.
/// Reads the top-level `{ "chapters": [ { "startTime": <secs>, "title": <str> } ] }`,
/// truncating `startTime` to whole seconds and skipping entries missing either
/// field. The body is streamed and capped at [`MAX_CHAPTERS_BYTES`] so an
/// oversized response is rejected mid-read rather than fully buffered first.
async fn fetch_remote_chapters(url: &str) -> anyhow::Result<Vec<RemoteChapter>> {
    let mut resp = download::chapters_client()
        .get(url)
        .send()
        .await?
        .error_for_status()?;

    // Reject early when the server advertises an oversized body.
    if let Some(len) = resp.content_length()
        && len > MAX_CHAPTERS_BYTES as u64
    {
        anyhow::bail!("chapters document too large (advertised {len} bytes)");
    }

    // Stream with the cap enforced as we go (chunked responses omit Content-Length).
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > MAX_CHAPTERS_BYTES {
            anyhow::bail!("chapters document exceeded {MAX_CHAPTERS_BYTES} bytes");
        }
        body.extend_from_slice(&chunk);
    }

    let doc: ChaptersDoc = serde_json::from_slice(&body)?;
    Ok(doc
        .chapters
        .into_iter()
        .filter_map(|c| {
            let title = c
                .title
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())?;
            let start = c.start_time?;
            if !start.is_finite() || start < 0.0 {
                return None;
            }
            Some(RemoteChapter {
                title,
                // Saturating float→int cast; truncates to whole seconds.
                starts_at_secs: start as i32,
            })
        })
        .collect())
}

/// Minimal view of a Podcasting 2.0 chapters JSON file — only the fields we keep.
#[derive(serde::Deserialize)]
struct ChaptersDoc {
    #[serde(default)]
    chapters: Vec<ChapterEntry>,
}

#[derive(serde::Deserialize)]
struct ChapterEntry {
    #[serde(rename = "startTime")]
    start_time: Option<f64>,
    title: Option<String>,
}

/// Add `new_episode_ids` (in feed order) to every playlist this podcast is
/// configured to auto-add to (`podcast_auto_playlist`). Where they land is per
/// link: `add_to_start` (falling back to the global `default_add_to_start`)
/// picks the start of the playlist (positions descending below the current min,
/// preserving feed order) or the end (continuing from the current max).
/// Newly-ingested episodes can't already be members, so plain inserts are
/// expected; a stray insert error is logged and skipped rather than aborting
/// the rest. Returns the number of (episode, playlist) links created.
async fn auto_add_to_playlists(
    dbc: &DatabaseConnection,
    podcast_id: i32,
    new_episode_ids: &[i32],
    default_add_to_start: bool,
) -> Result<usize, sea_orm::DbErr> {
    use halogen_orm::episode_playlist::{
        ActiveModel as EpPlActive, Column as EpPlCol, Entity as EpPlEntity,
    };
    use halogen_orm::podcast_auto_playlist::{Column as PapCol, Entity as PapEntity};

    if new_episode_ids.is_empty() {
        return Ok(0);
    }

    let targets = PapEntity::find()
        .filter(PapCol::PodcastId.eq(podcast_id))
        .all(dbc)
        .await?;
    if targets.is_empty() {
        return Ok(0);
    }

    let now = Utc::now();
    let mut created = 0usize;
    for link in targets {
        let playlist_id = link.playlist_id;
        let add_to_start = link.add_to_start.unwrap_or(default_add_to_start);

        // One transaction per target playlist so the position read and the
        // inserts that extend from it commit together — a concurrent writer can't
        // interleave between them. (Positions have no unique constraint, so a rare
        // collision is non-fatal and self-heals on the next reorder.)
        let txn = match dbc.begin().await {
            Ok(txn) => txn,
            Err(e) => {
                warn!("Auto-add: failed to open transaction for playlist {playlist_id}: {e}");
                continue;
            }
        };

        // One aggregate read (rather than loading every membership row): the end
        // path continues from the current max (mirrors the episode-playlist store
        // handler's `max + 1`); the start path places the batch wholly below the
        // current min (`min - n ..`), ascending so feed order is preserved —
        // positions may go negative, which sorts fine and is renumbered to 0..n
        // by the next reorder.
        let bound = if add_to_start {
            EpPlCol::Position.min()
        } else {
            EpPlCol::Position.max()
        };
        let bound_pos = match EpPlEntity::find()
            .filter(EpPlCol::PlaylistId.eq(playlist_id))
            .select_only()
            .column_as(bound, "bound_pos")
            .into_tuple::<Option<i32>>()
            .one(&txn)
            .await
        {
            Ok(v) => v.flatten(),
            Err(e) => {
                warn!("Auto-add: failed to read position bound for playlist {playlist_id}: {e}");
                continue;
            }
        };
        let mut next_pos = match bound_pos {
            Some(min) if add_to_start => min - new_episode_ids.len() as i32,
            Some(max) => max + 1,
            None => 0,
        };

        let mut local_created = 0usize;
        for &episode_id in new_episode_ids {
            let model = EpPlActive {
                episode_id: Set(episode_id),
                playlist_id: Set(playlist_id),
                position: Set(next_pos),
                created_at: Set(now),
                updated_at: Set(now),
            };
            match EpPlEntity::insert(model).exec(&txn).await {
                Ok(_) => {
                    local_created += 1;
                    next_pos += 1;
                }
                Err(e) => {
                    warn!(
                        "Auto-add: failed to add episode {} to playlist {}: {}",
                        episode_id, playlist_id, e
                    );
                }
            }
        }

        // Only count links that actually persisted (a rollback drops them all).
        match txn.commit().await {
            Ok(()) => created += local_created,
            Err(e) => warn!("Auto-add: failed to commit playlist {playlist_id}: {e}"),
        }
    }
    Ok(created)
}

/// Returns true when a podcast was polled more recently than `interval` ago. A
/// never-polled podcast or a clock skew (negative elapsed) is always "due".
fn not_due(now: DateTime<Utc>, podcast: &Model, interval: Duration) -> bool {
    podcast.polled_at.is_some_and(|last| {
        now.signed_duration_since(last)
            .to_std()
            .map(|elapsed| elapsed < interval)
            .unwrap_or(false)
    })
}

// ── Free-function entry points ───────────────────────────────────────────────
// Thin wrappers that build an [`RssManager`] and run it. The poller + tests call
// these rather than constructing the manager directly.

/// Sync all podcast feeds (legacy entry point): fetch every podcast (no interval
/// gating), no auto-download. Used by tests + sync-on-start.
pub async fn sync(
    dbc: &DatabaseConnection,
    max_concurrent: usize,
    no_sync_before: Option<DateTime<Utc>>,
) -> Result<(), String> {
    RssManager::new(
        dbc.clone(),
        SyncContext::legacy(max_concurrent, no_sync_before),
    )
    .run(None, |_| {})
    .await
}

/// Sync all podcasts using a [`SyncContext`] (per-podcast interval gating,
/// auto-download + retention). The scheduled poller's entry point.
pub async fn sync_with_context(dbc: &DatabaseConnection, ctx: &SyncContext) -> Result<(), String> {
    RssManager::new(dbc.clone(), ctx.clone())
        .run(None, |_| {})
        .await
}

/// Reports each podcast's outcome to `on_result` as its task finishes, optionally
/// restricted to `podcast_ids` (`None` = all feeds). Used by the on-demand
/// poll-job path so the
/// `halogen_polling::jobs::JobTracker` can stream per-feed
/// progress.
pub async fn sync_reported_with_context<F>(
    dbc: &DatabaseConnection,
    ctx: &SyncContext,
    podcast_ids: Option<Vec<i32>>,
    on_result: F,
) -> Result<(), String>
where
    F: Fn(PodcastPollResultData),
{
    RssManager::new(dbc.clone(), ctx.clone())
        .run(podcast_ids, on_result)
        .await
}
