//! Shared response types for the background polling control endpoints
//! (`/status`, `/poll`, `/start`, `/stop`) and the on-demand poll-job endpoints
//! (`/poll-job`, `/poll-job/{id}`, `/poll-jobs`).
//!
//! These live here (not in the server) so the API client and the server share a
//! single definition — the client deserialises exactly what the server emits.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// Response data for `GET /status` — whether the background poller is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollingStatusData {
    pub running: bool,
}

impl ResponsableData for PollingStatusData {}

/// Response data for the polling control operations (`/poll`, `/start`, `/stop`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollingOperationData {
    pub message: String,
}

impl ResponsableData for PollingOperationData {}

#[typeshare]
/// Lifecycle of an on-demand poll job tracked in server memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollJobStatus {
    /// The sync task is still walking feeds.
    Running,
    /// All feeds processed (some podcasts may still have had per-feed errors).
    Completed,
    /// The job aborted before completing (e.g. the DB query failed).
    Failed,
}

impl PollJobStatus {
    /// The snake_case token persisted in the DB `status` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            PollJobStatus::Running => "running",
            PollJobStatus::Completed => "completed",
            PollJobStatus::Failed => "failed",
        }
    }

    /// Parse the DB token; unknown values read as `Failed` (the conservative
    /// answer for a token we can't interpret).
    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "running" => PollJobStatus::Running,
            "completed" => PollJobStatus::Completed,
            _ => PollJobStatus::Failed,
        }
    }
}

#[typeshare]
/// What happened to a single podcast within a poll job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastPollOutcome {
    /// Feed fetched + parsed (may have yielded 0 new episodes).
    Polled,
    /// Server returned `304 Not Modified` — nothing re-parsed.
    Skipped,
    /// The feed fetch or parse failed.
    Error,
}

impl PodcastPollOutcome {
    /// The snake_case token persisted in the DB `outcome` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            PodcastPollOutcome::Polled => "polled",
            PodcastPollOutcome::Skipped => "skipped",
            PodcastPollOutcome::Error => "error",
        }
    }

    /// Parse the DB token; unknown values read as `Error`.
    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "polled" => PodcastPollOutcome::Polled,
            "skipped" => PodcastPollOutcome::Skipped,
            _ => PodcastPollOutcome::Error,
        }
    }
}

/// Map a `sync_podcast` outcome onto the reportable per-podcast row. `skipped`
/// is the 304-Not-Modified case (feed unchanged); a non-zero `errors` with no
/// new/updated counts marks the fetch/parse failure.
pub fn outcome_for(new: usize, updated: usize, errors: usize, skipped: bool) -> PodcastPollOutcome {
    if skipped {
        PodcastPollOutcome::Skipped
    } else if errors > 0 && new == 0 && updated == 0 {
        PodcastPollOutcome::Error
    } else {
        PodcastPollOutcome::Polled
    }
}

#[typeshare]
/// Per-podcast result row within a [`PollJobData`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodcastPollResultData {
    pub podcast_id: i32,
    pub title: String,
    pub outcome: PodcastPollOutcome,
    #[typeshare(serialized_as = "U53")]
    pub new_episodes: usize,
    #[typeshare(serialized_as = "U53")]
    pub updated_episodes: usize,
    #[typeshare(serialized_as = "U53")]
    pub errors: usize,
}

#[typeshare]
/// What started a poll job: an explicit user action, or the scheduled service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollJobTrigger {
    /// A user-triggered run (`POST /admin/poll-job`, pull-to-refresh hold).
    #[default]
    Manual,
    /// A tick of the background polling service.
    Scheduled,
}

impl PollJobTrigger {
    /// The snake_case token persisted in the DB `trigger` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            PollJobTrigger::Manual => "manual",
            PollJobTrigger::Scheduled => "scheduled",
        }
    }

    /// Parse the DB token; unknown values read as `Manual`.
    pub fn from_str_or_default(s: &str) -> Self {
        match s {
            "scheduled" => PollJobTrigger::Scheduled,
            _ => PollJobTrigger::Manual,
        }
    }
}

#[typeshare]
/// Snapshot of a poll job (`GET /admin/poll-job/{id}`, and items of
/// `GET /admin/poll-jobs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollJobData {
    #[typeshare(serialized_as = "U53")]
    pub id: u64,
    pub status: PollJobStatus,
    /// What started the run (defaulted for payloads predating the field).
    #[serde(default)]
    pub trigger: PollJobTrigger,
    /// `Some` = a single-podcast poll (podcast-detail page); `None` = all feeds.
    pub podcast_id: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub podcasts: Vec<PodcastPollResultData>,
    #[typeshare(serialized_as = "U53")]
    pub total_new: usize,
    #[typeshare(serialized_as = "U53")]
    pub total_updated: usize,
    #[typeshare(serialized_as = "U53")]
    pub total_errors: usize,
}

impl ResponsableData for PollJobData {}

#[typeshare]
/// Response data for `POST /poll-job` — the id to poll for status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollJobStartData {
    #[typeshare(serialized_as = "U53")]
    pub job_id: u64,
}

impl ResponsableData for PollJobStartData {}
