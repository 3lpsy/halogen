//! Import and validation at the durable mutation boundary.
use halogen_sync_enrich::OutboxOp;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedOperation {
    pub id: String,
    pub operation_json: String,
}

/// Decode the entire batch before writing any rows. Unknown operations stay with the caller.
pub fn decode_operations(
    operations: &[QueuedOperation],
) -> anyhow::Result<Vec<(String, OutboxOp)>> {
    anyhow::ensure!(operations.len() <= 100_000, "operation batch exceeds limit");
    operations
        .iter()
        .map(|operation| {
            anyhow::ensure!(
                !operation.id.is_empty() && operation.id.len() <= 256,
                "invalid operation id"
            );
            anyhow::ensure!(
                operation.operation_json.len() <= 1024 * 1024,
                "operation exceeds limit"
            );
            Ok((
                operation.id.clone(),
                serde_json::from_str(&operation.operation_json)?,
            ))
        })
        .collect()
}
