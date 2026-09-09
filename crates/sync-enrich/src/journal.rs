use crate::OutboxOp;
use serde::{Deserialize, Serialize};

/// Keeps rejected work available for review without blocking the pending FIFO.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    pub operation: OutboxOp,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default)]
    pub rejection: Option<String>,
}

impl JournalEntry {
    pub fn new(operation: OutboxOp) -> Self {
        Self {
            operation,
            source_id: None,
            attempts: 0,
            delivered: false,
            rejection: None,
        }
    }
}

/// Reads queue rows written before journals carried delivery metadata.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum StoredEntry {
    Journal(JournalEntry),
    Legacy(OutboxOp),
}

impl From<StoredEntry> for JournalEntry {
    fn from(entry: StoredEntry) -> Self {
        match entry {
            StoredEntry::Journal(entry) => entry,
            StoredEntry::Legacy(op) => Self::new(op),
        }
    }
}
