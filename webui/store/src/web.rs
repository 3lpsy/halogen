//! Per-user IndexedDB stores keep row-level JSON records and a durable auto-increment outbox. Schema bumps reset cached
//! entities, playback, selections, and sync metadata while preserving queued actions. Episodes expose indexed pod/pub
//! scalars for paging; unsupported filters scan JSON in memory. Row-level writes avoid whole-queue replacement on quota
//! failure.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use anyhow::Result;
use async_trait::async_trait;
use halogen_wire::{EpisodeData, PlaybackData, PlaylistData, PodcastData};
use idb::{Database, TransactionMode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use halogen_webui_platform::store_keys::{
    STORE_AUTO_PLAYLISTS, STORE_DB_PREFIX, STORE_EPISODES, STORE_OUTBOX, STORE_PLAYBACKS,
    STORE_PLAYLISTS, STORE_PODCASTS, STORE_SYNC_META,
};
use halogen_webui_store_idb::{
    IndexSpec, Schema, StoreKind, StoreSpec, clear_store, commit_tx, delete, get_all_episodes,
    get_all_json, get_all_with_keys, guarded, idb_err, num_key, one_store_tx, open, page_by_index,
    put_episode, put_json, scan_index_eq, scan_index_eq_ids, tx_done,
};

use super::{
    EpisodeOrder, EpisodeQuery, EpisodeQueryFilter, LocalStore, filter_count, filter_sort_paginate,
    outbox::OutboxOp,
};

/// Bump to evolve the schema on a new UI release. Cache stores reset automatically on bump; the `Durable` outbox
/// survives (see [`halogen_webui_store_idb`]). v3 adds delta metadata and auto-playlist snapshots. v2: the `episodes`
/// store became index-friendly `{ pod, pub, data }` objects with `idx_pub`/`idx_pod` indexes (the bump drops + rebuilds
/// the cache store for free).
const SCHEMA_VERSION: u32 = 3;

/// `episodes` index columns: the value-object props the indexes sort by, and the
/// index names. `pub` is the publish-time sort key (with [`UNDATED_SORT_KEY`] for
/// undated rows); `pod` is the podcast id.
const EP_PROP_PUB: &str = "pub";
const EP_PROP_POD: &str = "pod";
const EP_IDX_PUB: &str = "idx_pub";
const EP_IDX_POD: &str = "idx_pod";

/// Sort key for an undated episode — below any real Unix timestamp, so undated rows
/// land first under ascending (`Next`) and last under descending (`Prev`) `idx_pub`
/// iteration, exactly as `filter_sort_paginate` orders `None` `published_at`. Must
/// be finite: IndexedDB rejects `NaN`/`Infinity` as index keys.
const UNDATED_SORT_KEY: f64 = i64::MIN as f64;

/// The `idx_pub` sort key for an episode: its publish time in seconds, or the
/// undated sentinel.
fn pub_sort_key(ep: &EpisodeData) -> f64 {
    ep.published_at
        .map(|t| t.timestamp() as f64)
        .unwrap_or(UNDATED_SORT_KEY)
}

/// The object stores of `halogen.store.{segment}` (see the module docs).
const STORES: [StoreSpec; 7] = [
    StoreSpec {
        name: STORE_SYNC_META,
        auto_increment: false,
        kind: StoreKind::Cache,
        indexes: &[],
    },
    StoreSpec {
        name: STORE_AUTO_PLAYLISTS,
        auto_increment: false,
        kind: StoreKind::Cache,
        indexes: &[],
    },
    StoreSpec {
        name: STORE_PODCASTS,
        auto_increment: false,
        kind: StoreKind::Cache,
        indexes: &[],
    },
    StoreSpec {
        name: STORE_EPISODES,
        auto_increment: false,
        kind: StoreKind::Cache,
        // Index-friendly `{ pod, pub, data }` records (see module docs): `idx_pub`
        // pages the `published_at` feed, `idx_pod` scans one podcast's episodes.
        indexes: &[
            IndexSpec {
                name: EP_IDX_PUB,
                key_path: EP_PROP_PUB,
            },
            IndexSpec {
                name: EP_IDX_POD,
                key_path: EP_PROP_POD,
            },
        ],
    },
    StoreSpec {
        name: STORE_PLAYLISTS,
        auto_increment: false,
        kind: StoreKind::Cache,
        indexes: &[],
    },
    StoreSpec {
        name: STORE_PLAYBACKS,
        auto_increment: false,
        kind: StoreKind::Cache,
        indexes: &[],
    },
    StoreSpec {
        name: STORE_OUTBOX,
        auto_increment: true,
        kind: StoreKind::Durable,
        indexes: &[],
    },
];

/// IndexedDB-backed store, one database per active user. Constructed with the active user's namespace segment
/// (`"u{id}"` / `"anon"`) by `WorkerProvider`; the database is `halogen.store.{segment}`, so each user's cache + outbox
/// is isolated. (Audio blobs are *not* per-user, they stay a shared device cache in `halogen-webui-media`.)
pub struct WebLocalStore {
    /// `halogen.store.{segment}`.
    db_name: String,
    /// Lazily-opened connection (the provider constructs the store synchronously;
    /// IndexedDB opening is async). `Rc` because `idb::Database` isn't `Clone`; a
    /// racing double-open is harmless — both resolve to the same database.
    db: RefCell<Option<Rc<Database>>>,
}

impl WebLocalStore {
    /// Build a store for the namespace `segment` (typically `"u{id}"` / `"anon"`).
    pub fn new(segment: &str) -> Self {
        Self {
            db_name: format!("{STORE_DB_PREFIX}{segment}"),
            db: RefCell::new(None),
        }
    }

    async fn db(&self) -> Result<Rc<Database>> {
        if let Some(db) = self.db.borrow().as_ref() {
            return Ok(db.clone());
        }
        let schema = Schema {
            db_name: self.db_name.clone(),
            version: SCHEMA_VERSION,
            stores: &STORES,
        };
        let db = Rc::new(open(&schema).await?.db);
        *self.db.borrow_mut() = Some(db.clone());
        Ok(db)
    }

    /// Read + deserialize every record in `store_name` (bare-JSON-string stores:
    /// podcasts / playlists / playbacks). Episodes are wrapped objects — use
    /// [`Self::read_all_episodes`].
    async fn read_all<T: DeserializeOwned>(&self, store_name: &str) -> Result<Vec<T>> {
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, store_name, TransactionMode::ReadOnly)?;
        let out = get_all_json(&store).await?;
        tx_done(tx).await?;
        Ok(out)
    }

    /// Read every cached episode, unwrapping each `{ pod, pub, data }` record (the
    /// generic `read_all`/`get_all_json` only parse bare strings). The source for
    /// the in-memory fallback page/count and `episodes_by_ids`.
    async fn read_all_episodes(&self) -> Result<Vec<EpisodeData>> {
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_EPISODES, TransactionMode::ReadOnly)?;
        let out = get_all_episodes(&store).await?;
        tx_done(tx).await?;
        Ok(out)
    }

    /// Upsert a batch by `put`ting each record under `key_of(it)` in one
    /// transaction (IndexedDB `put` replaces by key — no read-modify-write).
    async fn put_all<T: Serialize>(
        &self,
        store_name: &str,
        items: &[T],
        key_of: impl Fn(&T) -> i32,
    ) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, store_name, TransactionMode::ReadWrite)?;
        for it in items {
            let key = num_key(key_of(it) as i64);
            put_json(&store, Some(&key), it).await?;
        }
        commit_tx(tx).await
    }

    /// Delete the rows keyed by `ids` (numeric out-of-line keys) in one transaction.
    async fn delete_keys(&self, store_name: &str, ids: &[i32]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, store_name, TransactionMode::ReadWrite)?;
        for id in ids {
            delete(&store, num_key(*id as i64)).await?;
        }
        commit_tx(tx).await
    }
}

#[async_trait(?Send)]
impl LocalStore for WebLocalStore {
    async fn sync_cursor(&self) -> Result<Option<String>> {
        Ok(self
            .read_all::<String>(STORE_SYNC_META)
            .await?
            .into_iter()
            .next())
    }
    async fn list_auto_playlists(
        &self,
    ) -> Result<std::collections::BTreeMap<i32, Vec<halogen_wire::PodcastAutoPlaylistData>>> {
        Ok(self
            .read_all::<(i32, Vec<halogen_wire::PodcastAutoPlaylistData>)>(STORE_AUTO_PLAYLISTS)
            .await?
            .into_iter()
            .collect())
    }

    async fn commit_changes(
        &self,
        changes: &halogen_sync_store::StoreChanges,
        operations: &[OutboxOp],
    ) -> Result<()> {
        let db = self.db().await?;
        let names = [
            STORE_PODCASTS,
            STORE_EPISODES,
            STORE_PLAYLISTS,
            STORE_PLAYBACKS,
            STORE_OUTBOX,
            STORE_SYNC_META,
            STORE_AUTO_PLAYLISTS,
        ];
        let tx = db
            .transaction(&names, TransactionMode::ReadWrite)
            .map_err(idb_err("tx"))?;
        let applied: Result<()> = async {
            if changes.check_sync_cursor {
                let metadata = tx.object_store(STORE_SYNC_META).map_err(idb_err("store"))?;
                let cursor = get_all_json::<String>(&metadata).await?.into_iter().next();
                anyhow::ensure!(
                    cursor == changes.expected_sync_cursor,
                    "sync cursor advanced during pull"
                );
            }
            if changes.require_empty_pending {
                let journal = tx.object_store(STORE_OUTBOX).map_err(idb_err("store"))?;
                for (_, row) in
                    get_all_with_keys::<halogen_sync_store::StoredEntry>(&journal).await?
                {
                    let entry = halogen_sync_store::JournalEntry::from(row);
                    anyhow::ensure!(
                        entry.rejection.is_some() || entry.delivered,
                        "pending changes prevent cache replacement"
                    );
                }
            }
            if changes.reset_cache {
                for name in [
                    STORE_PODCASTS,
                    STORE_EPISODES,
                    STORE_PLAYLISTS,
                    STORE_PLAYBACKS,
                    STORE_SYNC_META,
                    STORE_AUTO_PLAYLISTS,
                ] {
                    clear_store(&tx.object_store(name).map_err(idb_err("store"))?).await?;
                }
            }
            let metadata = tx.object_store(STORE_SYNC_META).map_err(idb_err("store"))?;
            if let Some(cursor) = &changes.sync_cursor {
                put_json(&metadata, Some(&num_key(0)), cursor).await?;
            }
            let auto = tx
                .object_store(STORE_AUTO_PLAYLISTS)
                .map_err(idb_err("store"))?;
            for id in &changes.deleted_auto_playlists {
                delete(&auto, num_key(*id as i64)).await?;
            }
            for (id, rows) in &changes.auto_playlists {
                put_json(&auto, Some(&num_key(*id as i64)), &(*id, rows)).await?;
            }

            for (name, ids) in [
                (STORE_PODCASTS, &changes.deleted_podcasts),
                (STORE_EPISODES, &changes.deleted_episodes),
                (STORE_PLAYLISTS, &changes.deleted_playlists),
                (STORE_PLAYBACKS, &changes.deleted_playbacks),
            ] {
                let store = tx.object_store(name).map_err(idb_err("store"))?;
                for id in ids {
                    delete(&store, num_key(*id as i64)).await?;
                }
            }
            let store = tx.object_store(STORE_PODCASTS).map_err(idb_err("store"))?;
            for row in &changes.podcasts {
                put_json(&store, Some(&num_key(row.id as i64)), row).await?;
            }
            let store = tx.object_store(STORE_EPISODES).map_err(idb_err("store"))?;
            for row in &changes.episodes {
                put_episode(
                    &store,
                    &num_key(row.id as i64),
                    &[
                        (EP_PROP_POD, row.podcast_id as f64),
                        (EP_PROP_PUB, pub_sort_key(row)),
                    ],
                    row,
                )
                .await?;
            }
            let store = tx.object_store(STORE_PLAYLISTS).map_err(idb_err("store"))?;
            for row in &changes.playlists {
                put_json(&store, Some(&num_key(row.id as i64)), row).await?;
            }
            let store = tx.object_store(STORE_PLAYBACKS).map_err(idb_err("store"))?;
            for row in &changes.playbacks {
                put_json(&store, Some(&num_key(row.episode_id as i64)), row).await?;
            }
            let store = tx.object_store(STORE_OUTBOX).map_err(idb_err("store"))?;
            for id in &changes.acknowledged_operations {
                delete(&store, num_key(*id as i64)).await?;
            }
            for operation in operations {
                put_json(&store, None, operation).await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = applied {
            let _ = tx.abort();
            return Err(error);
        }
        commit_tx(tx).await
    }

    async fn upsert_podcasts(&self, podcasts: &[PodcastData]) -> Result<()> {
        self.put_all(STORE_PODCASTS, podcasts, |p| p.id).await
    }

    async fn list_podcasts(&self) -> Result<Vec<PodcastData>> {
        self.read_all(STORE_PODCASTS).await
    }

    async fn upsert_episodes(&self, episodes: &[EpisodeData]) -> Result<()> {
        if episodes.is_empty() {
            return Ok(());
        }
        // Episode-specific upsert loop (the others use `put_all`): each row is
        // wrapped as `{ pod, pub, data }` so the `idx_pod`/`idx_pub` indexes have
        // scalar columns to sort by, beside the opaque DTO string in `data`.
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_EPISODES, TransactionMode::ReadWrite)?;
        for e in episodes {
            let key = num_key(e.id as i64);
            let scalars = [
                (EP_PROP_POD, e.podcast_id as f64),
                (EP_PROP_PUB, pub_sort_key(e)),
            ];
            put_episode(&store, &key, &scalars, e).await?;
        }
        commit_tx(tx).await
    }

    async fn list_episodes(&self, podcast_id: i32) -> Result<Vec<EpisodeData>> {
        // `idx_pod` equality scan instead of loading the whole store and filtering.
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_EPISODES, TransactionMode::ReadOnly)?;
        let out = scan_index_eq(&store, EP_IDX_POD, podcast_id as f64).await?;
        tx_done(tx).await?;
        Ok(out)
    }

    async fn list_episodes_page(&self, query: &EpisodeQuery) -> Result<Vec<EpisodeData>> {
        // Page unfiltered published-at order through idx_pub without scanning. Undated sentinels and primary-key ties
        // match in-memory ordering; filters and other sort orders still load and process the pool.
        if query.order_by == EpisodeOrder::PublishedAt && query.filter.is_empty() {
            let db = self.db().await?;
            let (tx, store) = one_store_tx(&db, STORE_EPISODES, TransactionMode::ReadOnly)?;
            let size = query.size.max(1) as usize;
            let offset = (query.page.max(0) as usize).saturating_mul(size);
            let out = page_by_index(&store, EP_IDX_PUB, query.descending, offset, size).await?;
            tx_done(tx).await?;
            return Ok(out);
        }
        Ok(filter_sort_paginate(self.read_all_episodes().await?, query))
    }

    async fn count_episodes(&self, filter: &EpisodeQueryFilter) -> Result<usize> {
        // Unfiltered → IndexedDB's O(1) store count; otherwise count the filtered
        // pool in memory (the predicate reads opaque-`data` fields a count can't
        // see). Mirrors native's COUNT fast path.
        if filter.is_empty() {
            let db = self.db().await?;
            let (tx, store) = one_store_tx(&db, STORE_EPISODES, TransactionMode::ReadOnly)?;
            let n = guarded(store.count(None).map_err(idb_err("count"))?)
                .await
                .map_err(idb_err("count"))?;
            tx_done(tx).await?;
            return Ok(n as usize);
        }
        Ok(filter_count(&self.read_all_episodes().await?, filter))
    }

    async fn episodes_by_ids(&self, ids: &[i32]) -> Result<Vec<EpisodeData>> {
        let want: HashSet<i32> = ids.iter().copied().collect();
        Ok(self
            .read_all_episodes()
            .await?
            .into_iter()
            .filter(|e| want.contains(&e.id))
            .collect())
    }

    async fn upsert_playlists(&self, playlists: &[PlaylistData]) -> Result<()> {
        self.put_all(STORE_PLAYLISTS, playlists, |p| p.id).await
    }

    async fn list_playlists(&self) -> Result<Vec<PlaylistData>> {
        self.read_all(STORE_PLAYLISTS).await
    }

    async fn delete_playlist_row(&self, playlist_id: i32) -> Result<()> {
        self.delete_keys(STORE_PLAYLISTS, &[playlist_id]).await
    }

    async fn save_playback(&self, playback: &PlaybackData) -> Result<()> {
        self.put_all(STORE_PLAYBACKS, std::slice::from_ref(playback), |p| {
            p.episode_id
        })
        .await
    }

    async fn list_playbacks(&self) -> Result<Vec<PlaybackData>> {
        self.read_all(STORE_PLAYBACKS).await
    }

    // ── Prune primitives ────────────────────────────────────────────────
    // The cascade order/policy lives in the `LocalStore` trait defaults; these are
    // single-store key deletes (each its own transaction, as before).

    async fn list_episode_ids_for_podcast(&self, podcast_id: i32) -> Result<Vec<i32>> {
        // `idx_pod` scan for ids only (primary keys — no value deserialization).
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_EPISODES, TransactionMode::ReadOnly)?;
        let ids = scan_index_eq_ids(&store, EP_IDX_POD, podcast_id as f64).await?;
        tx_done(tx).await?;
        Ok(ids.into_iter().map(|id| id as i32).collect())
    }

    async fn delete_episode_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.delete_keys(STORE_EPISODES, episode_ids).await
    }

    async fn delete_playback_rows(&self, episode_ids: &[i32]) -> Result<()> {
        self.delete_keys(STORE_PLAYBACKS, episode_ids).await
    }

    async fn delete_podcast_row(&self, podcast_id: i32) -> Result<()> {
        self.delete_keys(STORE_PODCASTS, &[podcast_id]).await
    }

    /// Append an op to the durable outbox — a single auto-increment `put`. Unlike
    /// the old localStorage backend (which rewrote the whole array and could drop
    /// the op on a quota error), this can't lose an already-persisted queue.
    async fn enqueue(&self, op: &OutboxOp) -> Result<()> {
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_OUTBOX, TransactionMode::ReadWrite)?;
        put_json(&store, None, op).await?;
        commit_tx(tx).await
    }

    async fn pending(&self) -> Result<Vec<(u64, OutboxOp)>> {
        Ok(self
            .journal_entries()
            .await?
            .into_iter()
            .filter(|(_, entry)| entry.rejection.is_none() && !entry.delivered)
            .map(|(id, entry)| (id, entry.operation))
            .collect())
    }

    async fn journal_entries(&self) -> Result<Vec<(u64, halogen_sync_store::JournalEntry)>> {
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_OUTBOX, TransactionMode::ReadOnly)?;
        let out: Vec<(u64, halogen_sync_store::StoredEntry)> = get_all_with_keys(&store).await?;
        tx_done(tx).await?;
        Ok(out
            .into_iter()
            .map(|(id, entry)| (id, entry.into()))
            .collect())
    }

    async fn save_journal_entry(
        &self,
        id: u64,
        entry: &halogen_sync_store::JournalEntry,
    ) -> Result<()> {
        anyhow::ensure!(id > 0 && id <= 9_007_199_254_740_991, "invalid journal id");
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_OUTBOX, TransactionMode::ReadWrite)?;
        put_json(&store, Some(&num_key(id as i64)), entry).await?;
        commit_tx(tx).await
    }

    async fn ack(&self, op_id: u64) -> Result<()> {
        let db = self.db().await?;
        let (tx, store) = one_store_tx(&db, STORE_OUTBOX, TransactionMode::ReadWrite)?;
        delete(&store, num_key(op_id as i64)).await?;
        commit_tx(tx).await
    }

    async fn clear(&self) -> Result<()> {
        let db = self.db().await?;
        let names = [
            STORE_PODCASTS,
            STORE_EPISODES,
            STORE_PLAYLISTS,
            STORE_PLAYBACKS,
            STORE_OUTBOX,
            STORE_SYNC_META,
            STORE_AUTO_PLAYLISTS,
        ];
        let tx = db
            .transaction(&names, TransactionMode::ReadWrite)
            .map_err(idb_err("tx"))?;
        for name in names {
            let store = tx.object_store(name).map_err(idb_err("store"))?;
            clear_store(&store).await?;
        }
        commit_tx(tx).await
    }
}
