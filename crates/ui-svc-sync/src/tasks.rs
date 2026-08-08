//! Detached one-shot/poll tasks the worker spawns (via `ensure_*` / the command
//! router) and that report back through the internal `Command` channel — kept out
//! of the command-router module so all the fire-and-forget `!Send` fetches live in
//! one place. Co-located here: the device-download task lives in `download.rs`
//! (its own larger concern).

use futures::channel::mpsc::UnboundedSender;
use halogen_api::ApiClient;
use halogen_wire::{DefaultListParams, EpisodeInclude, FilterParams, Pagination, PodcastInclude};

use super::{Command, sleep_secs};
use halogen_ui_logging::debug;

/// Detached task: fetch one podcast by id and report it back via
/// [`Command::PodcastFetched`] (spawned by `ensure_podcast`). `None` on failure
/// so the worker clears the in-flight guard either way.
pub(super) async fn fetch_podcast_task(
    podcast_id: i32,
    api: ApiClient,
    tx: UnboundedSender<Command>,
) {
    let podcast = api.get_podcast(podcast_id).await.ok();
    let _ = tx.unbounded_send(Command::PodcastFetched {
        podcast_id,
        podcast,
    });
}

/// Detached task: fetch a BATCH of podcasts by id in one `list_podcasts` request
/// (with `PodcastConfig` included) and report them back via
/// [`Command::PodcastsFetched`] (spawned by `ensure_podcasts_batch`). Collapses the
/// per-episode-row `EnsurePodcast` N+1 into a single round trip. `None` on failure
/// so the worker clears every requested in-flight guard either way.
pub(super) async fn fetch_podcasts_task(
    podcast_ids: Vec<i32>,
    api: ApiClient,
    tx: UnboundedSender<Command>,
) {
    let params = DefaultListParams::<PodcastInclude> {
        // `size` must cover the whole id set or the server pages it (default 10).
        pagination: Some(Pagination {
            page: 0,
            size: podcast_ids.len() as i32,
        }),
        order: None,
        includes: Some(vec![PodcastInclude::PodcastConfig]),
        filter: Some(FilterParams {
            ids: Some(podcast_ids.clone()),
            ..Default::default()
        }),
    };
    let podcasts = match api.list_podcasts(params).await {
        Ok(page) => Some(page.data),
        Err(e) => {
            debug!(?podcast_ids, error = %e, "batched podcast prime failed");
            None
        }
    };
    let _ = tx.unbounded_send(Command::PodcastsFetched {
        podcast_ids,
        podcasts,
    });
}

/// Detached task: fetch one episode WITH its chapters and report it back via
/// [`Command::EpisodeChaptersFetched`] (spawned by `ensure_episode_chapters`).
/// `None` on failure so the worker clears the in-flight guard either way.
pub(super) async fn fetch_episode_chapters_task(
    episode_id: i32,
    api: ApiClient,
    tx: UnboundedSender<Command>,
) {
    let episode = api
        .get_episode(episode_id, &[EpisodeInclude::Chapters])
        .await
        .ok();
    let _ = tx.unbounded_send(Command::EpisodeChaptersFetched {
        episode_id,
        episode,
    });
}

/// Server-download progress poll: ≈1 Hz, capped so a stuck poll can't run forever
/// (a large file is minutes; the loop also exits as soon as the server reports the
/// download finished). The grace covers the gap before the server starts fetching
/// and a transient blip mid-poll.
const SERVER_PROGRESS_POLL_TICKS: u32 = 1800; // ~30 min ceiling (1 Hz while active)
const SERVER_PROGRESS_START_GRACE_SECS: u32 = 30; // wall-clock 404/err budget before giving up
const SERVER_PROGRESS_MAX_BACKOFF_SECS: u32 = 8; // cap on the idle-phase backoff

/// Detached task: poll one episode's SERVER-download progress ≈1 Hz and mirror it
/// into `server_download_progress` (a filling ring) via
/// [`Command::ServerDownloadProgress`], then refresh the durable status when it
/// finishes via [`Command::ServerDownloadComplete`].
///
/// The progress endpoint 404s both before the server starts and after it finishes;
/// we tell the two apart by whether we'd already seen progress (`seen`). Percent
/// emissions are throttled to integer changes so a slow download doesn't republish
/// `EpisodeState` every tick.
pub(super) async fn server_download_poll_task(
    episode_id: i32,
    api: ApiClient,
    tx: UnboundedSender<Command>,
    offline: std::rc::Rc<std::cell::Cell<bool>>,
) {
    let mut seen = false;
    // Wall-clock seconds spent waiting on a 404/err before the download starts, and
    // the current inter-poll delay. While bytes flow we poll at 1 Hz for responsive
    // progress; before the server starts (or during a blip) we back off
    // exponentially so a not-yet-started download isn't hammered once per second for
    // the whole grace window (the 404 storm Lighthouse flagged).
    let mut idle_secs = 0u32;
    let mut backoff = 1u32;
    let mut last_pct: i32 = -1;
    for _ in 0..SERVER_PROGRESS_POLL_TICKS {
        sleep_secs(backoff as u64).await;
        // Manual "Go Offline": stop hitting the server; the poll re-arms on the
        // next download start after reconnect.
        if offline.get() {
            return;
        }
        match api.get_download_progress(episode_id).await {
            Ok(Some(p)) => {
                seen = true;
                idle_secs = 0;
                backoff = 1; // responsive 1 Hz while bytes flow
                // `percent` is `None` for unknown-length (chunked) responses —
                // leave the ring absent so the control shows a spinner instead.
                if let Some(frac) = p.percent {
                    let pct = (frac.clamp(0.0, 1.0) * 100.0).round() as i32;
                    if pct != last_pct {
                        last_pct = pct;
                        let _ = tx.unbounded_send(Command::ServerDownloadProgress {
                            episode_id,
                            percent: pct as u8,
                        });
                    }
                }
            }
            // 404: not running. Finished if we'd seen it in flight; otherwise the
            // server may not have started yet — wait out a bounded grace period,
            // backing off between probes.
            Ok(None) => {
                if seen {
                    break;
                }
                idle_secs += backoff;
                if idle_secs >= SERVER_PROGRESS_START_GRACE_SECS {
                    break;
                }
                backoff = (backoff * 2).min(SERVER_PROGRESS_MAX_BACKOFF_SECS);
            }
            // Transient (offline blip): keep trying within the grace budget, backing
            // off the same way.
            Err(e) => {
                debug!(episode_id, error = %e, "server progress poll failed");
                idle_secs += backoff;
                if idle_secs >= SERVER_PROGRESS_START_GRACE_SECS {
                    break;
                }
                backoff = (backoff * 2).min(SERVER_PROGRESS_MAX_BACKOFF_SECS);
            }
        }
    }
    // Refresh the durable status (Downloaded / DownloadError / …) and clear the
    // ring. Include Playback so re-caching the body keeps the embedded cursor.
    // Retry a few times: a transient offline blip at the very end would otherwise
    // hand back `None`, and the handler would leave the status stuck on the
    // optimistic `Downloading` with no further poll to reconcile it.
    let mut episode = None;
    for attempt in 0..3 {
        if attempt > 0 {
            sleep_secs(2).await;
        }
        match api
            .get_episode(episode_id, &[EpisodeInclude::Playback])
            .await
        {
            Ok(ep) => {
                episode = Some(ep);
                break;
            }
            Err(e) => {
                debug!(episode_id, attempt, error = %e, "final status refresh failed; retrying")
            }
        }
    }
    let _ = tx.unbounded_send(Command::ServerDownloadComplete {
        episode_id,
        episode,
    });
}
