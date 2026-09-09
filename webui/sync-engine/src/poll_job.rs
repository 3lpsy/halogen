//! Server poll-job orchestration: start a poll job, poll its status to completion, refresh the list, and toast a
//! summary. The pull-to-refresh gesture hook (`halogen_webui_state`'s `use_pull_to_refresh`) spawns [`run_poll_job`]
//! when an admin holds the pull past the countdown. Keeping the orchestration here keeps that hook focused on the
//! gesture/visual phase and lets the orchestration be tested/reused independently.

use dioxus::prelude::*;
use halogen_wire::{PodcastPollOutcome, PollJobData, PollJobStatus};

use halogen_webui_component_toast::ToastHandle;
use halogen_webui_component_toast::{ToastDecision, ToastPolicy, classify};
use halogen_webui_config::ClientConfig;
use halogen_webui_logging::warn;
use halogen_webui_platform::time::sleep_ms;

/// Visual phase of the pull-to-refresh gesture, rendered by the indicator and
/// driven by the poll orchestration here. Lives in the sync worker (not the
/// gesture hook) so [`run_poll_job`] and the hook can both name it across crate
/// bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PullPhase {
    /// No gesture in progress — the indicator is hidden.
    Idle,
    /// Dragging at the top; the `f64` is the pulled offset in px (reveal height).
    Pulling(f64),
    /// Past the threshold; the `i32` is the seconds left before a poll fires.
    Armed(i32),
    /// Client-side refresh in flight (shown briefly).
    Refreshing,
    /// Server poll job running.
    Polling,
}

impl PullPhase {
    /// Phases the gesture stream must not overwrite — once a refresh/poll is in
    /// flight, further `pull`/`reset` messages from a trailing gesture are ignored
    /// until the in-flight work clears the phase back to `Idle`.
    pub fn is_busy(self) -> bool {
        matches!(self, PullPhase::Refreshing | PullPhase::Polling)
    }
}

/// Start a server poll job, poll its status to completion, then refresh the list
/// and toast a summary. Always clears `phase` back to `Idle` when it returns.
pub async fn run_poll_job(
    config: Signal<ClientConfig>,
    toast: ToastHandle,
    podcast_id: Option<i32>,
    on_refresh: Callback<()>,
    mut phase: Signal<PullPhase>,
) {
    // `api_client()` folds "no server URL" and manual "Go Offline" into one
    // `None`; disambiguate here so offline mode doesn't claim no server exists.
    if config.peek().manual_offline {
        toast.error("You're in offline mode — go online to sync the server.");
        phase.set(PullPhase::Idle);
        return;
    }
    let Some(api) = config.peek().api_client() else {
        toast.error("No server configured.");
        phase.set(PullPhase::Idle);
        return;
    };

    let start = match api.start_poll_job(podcast_id).await {
        Ok(s) => s,
        Err(e) => {
            // Route through the shared funnel so a 5xx is scrubbed to a generic
            // message (no internals leak). This is an explicit user action, so give
            // it a friendly line for the cases the funnel handles silently.
            match classify(&e, ToastPolicy::Foreground) {
                ToastDecision::Offline => toast.error("You're offline — can't reach the server."),
                ToastDecision::SignOut => toast.error("Session expired — please sign in again."),
                decided => {
                    toast.apply(decided);
                }
            }
            phase.set(PullPhase::Idle);
            return;
        }
    };

    // Poll status until terminal or a generous timeout (~3 min at 1.5s steps).
    let mut last: Option<PollJobData> = None;
    let mut errors_in_a_row = 0;
    for _ in 0..120 {
        sleep_ms(1500).await;
        match api.get_poll_job(start.job_id).await {
            Ok(job) => {
                errors_in_a_row = 0;
                let running = matches!(job.status, PollJobStatus::Running);
                last = Some(job);
                if !running {
                    break;
                }
            }
            Err(e) => {
                warn!(error = %e, "Poll-job status check failed");
                // A dead token won't recover, and a run of failures means we've
                // lost the server — stop checking instead of warning for the full
                // ~3 min. The job keeps running server-side; the list still
                // refreshes below.
                errors_in_a_row += 1;
                if errors_in_a_row >= 5
                    || matches!(
                        classify(&e, ToastPolicy::Background),
                        ToastDecision::SignOut
                    )
                {
                    break;
                }
            }
        }
    }

    // Show whatever landed; refresh the list regardless so newly-ingested
    // episodes appear.
    on_refresh.call(());
    match last {
        Some(job) if matches!(job.status, PollJobStatus::Completed) => {
            let polled = job
                .podcasts
                .iter()
                .filter(|p| !matches!(p.outcome, PodcastPollOutcome::Skipped))
                .count();
            if job.total_new == 0 {
                toast.info(format!("No new episodes ({polled} checked)"));
            } else {
                toast.success(format!(
                    "{} new episode{} across {} podcast{}",
                    job.total_new,
                    plural(job.total_new),
                    polled,
                    plural(polled),
                ));
            }
        }
        Some(job) if matches!(job.status, PollJobStatus::Failed) => {
            toast.error("Server poll failed");
        }
        _ => toast.info("Server poll still running — refreshed what's ready"),
    }

    phase.set(PullPhase::Idle);
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
