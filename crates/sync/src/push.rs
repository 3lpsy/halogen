use halogen_apiclient::{ApiClient, ApiError};
use halogen_sync_policy::{ATTEMPT_BUDGET, is_countable_failure, is_permanent_failure};
use halogen_sync_store::{JournalEntry, LocalStore};
use halogen_wire::PodcastData;

pub enum PushOutcome {
    Applied(Option<Box<PodcastData>>),
    Retry { error: ApiError, attempts: u32 },
    Quarantined(ApiError),
}

/// A failed acknowledgement leaves the operation queued for idempotent replay.
pub async fn push_entry(
    store: &dyn LocalStore,
    api: &ApiClient,
    id: u64,
    mut entry: JournalEntry,
) -> anyhow::Result<PushOutcome> {
    anyhow::ensure!(
        entry.rejection.is_none() && !entry.delivered,
        "entry is not pending"
    );
    match entry.operation.apply(api).await {
        Ok(created) => {
            complete(store, id, &mut entry).await?;
            Ok(PushOutcome::Applied(created.map(Box::new)))
        }
        Err(error)
            if entry.operation.is_absence_request()
                && halogen_sync_policy::status_of_error(&error) == Some(404) =>
        {
            complete(store, id, &mut entry).await?;
            Ok(PushOutcome::Applied(None))
        }
        Err(error) => {
            if is_countable_failure(&error) {
                entry.attempts = entry.attempts.saturating_add(1);
            }
            if is_permanent_failure(&error) || entry.attempts >= ATTEMPT_BUDGET {
                entry.rejection = Some(error.to_string());
                store.save_journal_entry(id, &entry).await?;
                Ok(PushOutcome::Quarantined(error))
            } else {
                store.save_journal_entry(id, &entry).await?;
                Ok(PushOutcome::Retry {
                    error,
                    attempts: entry.attempts,
                })
            }
        }
    }
}

async fn complete(store: &dyn LocalStore, id: u64, entry: &mut JournalEntry) -> anyhow::Result<()> {
    if entry.source_id.is_some() {
        entry.delivered = true;
        store.save_journal_entry(id, entry).await
    } else {
        store.ack(id).await
    }
}
