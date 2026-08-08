use dioxus::prelude::*;
use halogen_wire::{PodcastPollOutcome, PollJobData, PollJobStatus};

use crate::components::{BackButton, resource_list_view};
use halogen_ui_state::hooks::use_config;

/// Server polling history page (admin-only; guarded by nav + route).
///
/// Surfaces the server's in-memory poll-job history (the last ~10 on-demand
/// polls triggered by pull-to-refresh), with per-podcast outcomes.
#[component]
pub fn Polling() -> Element {
    let cfg = use_config();

    // Fetch the recent poll jobs (re-runs if the session changes; `restart` on the
    // Refresh button re-pulls on demand).
    let mut jobs = use_resource(move || {
        // `api_client()` returns None when unconfigured OR manually offline, so
        // "Go Offline" suppresses this fetch (the resource just shows the message).
        let client = cfg.read().api_client_or_err();
        async move {
            let client = client?;
            client.list_poll_jobs().await.map_err(|e| e.to_string())
        }
    });

    rsx! {
        div { class: "p-2 space-y-6",
            BackButton {}
            div { class: "flex items-center justify-between",
                h1 { class: "text-3xl font-bold", "Server Polling History" }
                button {
                    class: "btn btn-ghost btn-sm",
                    onclick: move |_| jobs.restart(),
                    "Refresh"
                }
            }

            h2 { class: "text-xl font-semibold", "Poll history" }
            p { class: "text-sm text-muted",
                "On-demand and scheduled feed polls (newest first). Stored in the server's database; the most recent 50 are kept."
            }

            {
                resource_list_view(
                    &*jobs.read_unchecked(),
                    "poll history",
                    "No polls yet — pull a list down and hold to trigger one.",
                    "flex flex-col gap-3",
                    |job: PollJobData| rsx! {
                        PollJobCard { key: "{job.id}", job }
                    },
                )
            }
        }
    }
}

#[component]
fn PollJobCard(job: PollJobData) -> Element {
    let (badge, label) = match job.status {
        PollJobStatus::Running => ("badge-info", "Running"),
        PollJobStatus::Completed => ("badge-success", "Completed"),
        PollJobStatus::Failed => ("badge-error", "Failed"),
    };
    let scope = match job.podcast_id {
        Some(id) => format!("Podcast #{id}"),
        None => "All feeds".to_string(),
    };
    // Scheduled service ticks are persisted alongside manual runs now — badge
    // the source so the history reads at a glance.
    let trigger = match job.trigger {
        halogen_wire::PollJobTrigger::Manual => "Manual",
        halogen_wire::PollJobTrigger::Scheduled => "Scheduled",
    };

    rsx! {
        div { class: "card bg-base-100 border border-base-300",
            div { class: "card-body gap-2 p-4",
                div { class: "flex items-center gap-2 flex-wrap",
                    span { class: "badge {badge}", "{label}" }
                    span { class: "badge badge-ghost", "{trigger}" }
                    span { class: "font-semibold", "{scope}" }
                    span { class: "text-xs text-muted", "started {job.started_at}" }
                }
                div { class: "text-sm",
                    span { class: "text-success", "{job.total_new} new" }
                    ", "
                    span { "{job.total_updated} updated" }
                    ", "
                    span { class: "text-error", "{job.total_errors} errors" }
                    " across {job.podcasts.len()} feed(s)"
                }
                if !job.podcasts.is_empty() {
                    details { class: "text-sm",
                        summary { class: "cursor-pointer text-muted", "Per-podcast detail" }
                        div { class: "mt-2 flex flex-col gap-1",
                            for p in job.podcasts.clone() {
                                div { class: "flex items-center gap-2 text-xs",
                                    span {
                                        class: "badge badge-sm {outcome_badge(p.outcome)}",
                                        "{outcome_label(p.outcome)}"
                                    }
                                    span { class: "truncate flex-1", "{p.title}" }
                                    span { class: "text-muted",
                                        "{p.new_episodes} new / {p.updated_episodes} upd / {p.errors} err"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn outcome_badge(o: PodcastPollOutcome) -> &'static str {
    match o {
        PodcastPollOutcome::Polled => "badge-success",
        PodcastPollOutcome::Skipped => "badge-ghost",
        PodcastPollOutcome::Error => "badge-error",
    }
}

fn outcome_label(o: PodcastPollOutcome) -> &'static str {
    match o {
        PodcastPollOutcome::Polled => "Polled",
        PodcastPollOutcome::Skipped => "Skipped",
        PodcastPollOutcome::Error => "Error",
    }
}
