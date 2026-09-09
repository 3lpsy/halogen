use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use anyhow::{Result, bail};
use halogen_sync_store::{EpisodeQuery, LocalStore, OutboxOp, StoreChanges};
use halogen_wire::EpisodeData;

/// Stages one command's metadata and journal writes until its atomic commit.
pub struct BufferedStore {
    pub(super) inner: Rc<dyn LocalStore>,
    pub(super) changes: RefCell<StoreChanges>,
    pub(super) operations: RefCell<Vec<Option<OutboxOp>>>,
    read_error: RefCell<Option<String>>,
}

impl BufferedStore {
    pub fn new(inner: Rc<dyn LocalStore>) -> Self {
        Self {
            inner,
            changes: RefCell::default(),
            operations: RefCell::default(),
            read_error: RefCell::default(),
        }
    }

    pub async fn commit(&self) -> Result<()> {
        if let Some(error) = self.read_error.borrow().as_ref() {
            bail!("cannot commit after a failed cache read: {error}");
        }
        let changes = self.changes.borrow().clone();
        let operations: Vec<_> = self.operations.borrow().iter().flatten().cloned().collect();
        self.inner.commit_changes(&changes, &operations).await
    }

    pub(super) fn read<T>(&self, result: Result<T>) -> Result<T> {
        if let Err(error) = &result {
            *self.read_error.borrow_mut() = Some(error.to_string());
        }
        result
    }

    pub(super) async fn episodes(&self) -> Result<Vec<EpisodeData>> {
        let query = EpisodeQuery {
            size: i32::MAX,
            ..Default::default()
        };
        let rows = self.read(self.inner.list_episodes_page(&query).await)?;
        let changes = self.changes.borrow();
        Ok(overlay(
            rows,
            &changes.episodes,
            &changes.deleted_episodes,
            |row| row.id,
        ))
    }
}

pub(super) fn overlay<T: Clone>(
    rows: Vec<T>,
    writes: &[T],
    deleted: &[i32],
    key: impl Fn(&T) -> i32,
) -> Vec<T> {
    let mut rows: BTreeMap<_, _> = rows.into_iter().map(|row| (key(&row), row)).collect();
    rows.extend(writes.iter().map(|row| (key(row), row.clone())));
    for id in deleted {
        rows.remove(id);
    }
    rows.into_values().collect()
}
