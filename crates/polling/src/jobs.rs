//! Persist each scheduled or manual feed-sync run and its per-podcast outcomes. Finishing a job prunes history beyond
//! MAX_JOBS, with outcome rows removed by cascade.

use chrono::Utc;
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use tracing::warn;

use halogen_orm::{poll_job, poll_job_podcast};
use halogen_wire::{
    PodcastPollOutcome, PodcastPollResultData, PollJobData, PollJobStatus, PollJobTrigger,
};

/// How many jobs the history retains. Older rows are pruned on each finish
/// (their outcome rows cascade).
pub const MAX_JOBS: u64 = 50;

/// How many jobs `recent()` returns (each with its full outcome list).
const RECENT_JOBS: u64 = 10;

/// Durable poll-job history. Cheap to clone-share behind an `Arc` (it holds only
/// the DB handle); every method is best-effort against the DB — a failed write
/// is logged, never propagated into the sync itself.
#[derive(Debug)]
pub struct JobTracker {
    dbc: DatabaseConnection,
}

impl JobTracker {
    pub fn new(dbc: DatabaseConnection) -> Self {
        Self { dbc }
    }

    /// Persist a new running job and return its id. `podcast_id` scopes the job
    /// to a single feed (`None` = all feeds).
    pub async fn create(
        &self,
        trigger: PollJobTrigger,
        podcast_id: Option<i32>,
    ) -> Result<u64, String> {
        let now = Utc::now();
        let row = poll_job::ActiveModel {
            id: NotSet,
            status: Set(PollJobStatus::Running.as_str().to_string()),
            trigger: Set(trigger.as_str().to_string()),
            podcast_id: Set(podcast_id),
            started_at: Set(now),
            completed_at: Set(None),
            total_new: Set(0),
            total_updated: Set(0),
            total_errors: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.dbc)
        .await
        .map_err(|e| format!("Failed to create poll job: {e}"))?;
        Ok(row.id as u64)
    }

    /// Persist a per-podcast result and fold it into the job's totals.
    /// Best-effort: a DB failure is logged and swallowed so it can never abort
    /// the sync that's reporting.
    pub async fn record_podcast(&self, job_id: u64, result: PodcastPollResultData) {
        let now = Utc::now();
        let insert = poll_job_podcast::ActiveModel {
            id: NotSet,
            poll_job_id: Set(job_id as i32),
            podcast_id: Set(result.podcast_id),
            title: Set(result.title.clone()),
            outcome: Set(result.outcome.as_str().to_string()),
            new_episodes: Set(result.new_episodes as i32),
            updated_episodes: Set(result.updated_episodes as i32),
            errors: Set(result.errors as i32),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&self.dbc)
        .await;
        if let Err(e) = insert {
            warn!(job_id, error = %e, "Failed to persist poll-job podcast outcome");
            return;
        }
        // Fold the totals onto the job row (read-modify-write is fine: one job is
        // written by one sync task; readers only see monotonic progress).
        match poll_job::Entity::find_by_id(job_id as i32)
            .one(&self.dbc)
            .await
        {
            Ok(Some(job)) => {
                let mut active: poll_job::ActiveModel = job.clone().into();
                active.total_new = Set(job.total_new + result.new_episodes as i32);
                active.total_updated = Set(job.total_updated + result.updated_episodes as i32);
                active.total_errors = Set(job.total_errors + result.errors as i32);
                if let Err(e) = active.update(&self.dbc).await {
                    warn!(job_id, error = %e, "Failed to update poll-job totals");
                }
            }
            Ok(None) => warn!(job_id, "Poll job vanished while recording an outcome"),
            Err(e) => warn!(job_id, error = %e, "Failed to load poll job for totals"),
        }
    }

    /// Mark a job finished (`Completed` unless the whole sync aborted), then
    /// prune history past [`MAX_JOBS`]. Best-effort like `record_podcast`.
    pub async fn finish(&self, job_id: u64, status: PollJobStatus) {
        match poll_job::Entity::find_by_id(job_id as i32)
            .one(&self.dbc)
            .await
        {
            Ok(Some(job)) => {
                let mut active: poll_job::ActiveModel = job.into();
                active.status = Set(status.as_str().to_string());
                active.completed_at = Set(Some(Utc::now()));
                if let Err(e) = active.update(&self.dbc).await {
                    warn!(job_id, error = %e, "Failed to finish poll job");
                }
            }
            Ok(None) => warn!(job_id, "Poll job vanished before finish"),
            Err(e) => warn!(job_id, error = %e, "Failed to load poll job to finish"),
        }
        self.prune().await;
    }

    /// Delete jobs beyond the newest [`MAX_JOBS`] (children cascade).
    async fn prune(&self) {
        let keep: Vec<i32> = match poll_job::Entity::find()
            .order_by_desc(poll_job::Column::Id)
            .limit(MAX_JOBS)
            .all(&self.dbc)
            .await
        {
            Ok(rows) => rows.into_iter().map(|r| r.id).collect(),
            Err(e) => {
                warn!(error = %e, "Failed to load poll jobs for pruning");
                return;
            }
        };
        if keep.len() < MAX_JOBS as usize {
            return; // Fewer than the cap exist — nothing to prune.
        }
        let deleted = poll_job::Entity::delete_many()
            .filter(poll_job::Column::Id.is_not_in(keep))
            .exec(&self.dbc)
            .await;
        if let Err(e) = deleted {
            warn!(error = %e, "Failed to prune poll-job history");
        }
    }

    /// Snapshot of one job (with its per-podcast outcomes), if still retained.
    pub async fn get(&self, job_id: u64) -> Option<PollJobData> {
        let job = poll_job::Entity::find_by_id(job_id as i32)
            .one(&self.dbc)
            .await
            .map_err(|e| warn!(job_id, error = %e, "Failed to load poll job"))
            .ok()??;
        let mut outcomes = self.outcomes_for(&[job.id]).await;
        let own = outcomes.remove(&job.id).unwrap_or_default();
        Some(to_wire(job, own))
    }

    /// Snapshot of the retained jobs, newest first (capped at [`RECENT_JOBS`]),
    /// each with its full outcome list.
    pub async fn recent(&self) -> Vec<PollJobData> {
        let jobs = match poll_job::Entity::find()
            .order_by_desc(poll_job::Column::Id)
            .limit(RECENT_JOBS)
            .all(&self.dbc)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %e, "Failed to load recent poll jobs");
                return Vec::new();
            }
        };
        let ids: Vec<i32> = jobs.iter().map(|j| j.id).collect();
        let mut outcomes = self.outcomes_for(&ids).await;
        jobs.into_iter()
            .map(|job| {
                let own = outcomes.remove(&job.id).unwrap_or_default();
                to_wire(job, own)
            })
            .collect()
    }

    /// Batch-load the outcome rows for a set of jobs, keyed by job id and in
    /// insertion (row-id) order within each job.
    async fn outcomes_for(
        &self,
        job_ids: &[i32],
    ) -> std::collections::HashMap<i32, Vec<poll_job_podcast::Model>> {
        let mut by_job: std::collections::HashMap<i32, Vec<poll_job_podcast::Model>> =
            std::collections::HashMap::new();
        if job_ids.is_empty() {
            return by_job;
        }
        let rows = match poll_job_podcast::Entity::find()
            .filter(poll_job_podcast::Column::PollJobId.is_in(job_ids.iter().copied()))
            .order_by_asc(poll_job_podcast::Column::Id)
            .all(&self.dbc)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!(error = %e, "Failed to load poll-job outcomes");
                return by_job;
            }
        };
        for row in rows {
            by_job.entry(row.poll_job_id).or_default().push(row);
        }
        by_job
    }
}

/// Convert with the same argument order everywhere: overflow-safe (`max(0)`)
/// because the DB columns are written from `usize` counts.
fn to_usize(v: i32) -> usize {
    v.max(0) as usize
}

/// Map a job row + its outcome rows onto the wire snapshot.
fn to_wire(job: poll_job::Model, outcomes: Vec<poll_job_podcast::Model>) -> PollJobData {
    PollJobData {
        id: job.id as u64,
        status: PollJobStatus::from_str_or_default(&job.status),
        trigger: PollJobTrigger::from_str_or_default(&job.trigger),
        podcast_id: job.podcast_id,
        started_at: job.started_at,
        completed_at: job.completed_at,
        podcasts: outcomes
            .into_iter()
            .map(|o| PodcastPollResultData {
                podcast_id: o.podcast_id,
                title: o.title,
                outcome: PodcastPollOutcome::from_str_or_default(&o.outcome),
                new_episodes: to_usize(o.new_episodes),
                updated_episodes: to_usize(o.updated_episodes),
                errors: to_usize(o.errors),
            })
            .collect(),
        total_new: to_usize(job.total_new),
        total_updated: to_usize(job.total_updated),
        total_errors: to_usize(job.total_errors),
    }
}

// `outcome_for` lives in `halogen_wire` (pure classifier on the wire enum) so
// the rss crate can use it without depending on polling.
