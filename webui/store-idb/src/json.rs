//! Store DTOs as JSON strings under explicit keys, avoiding serde-wasm-bindgen. Plain strings require in-memory
//! filtering; indexed helpers wrap them as `{ <key paths>, data }` with scalar properties exposed to IndexedDB,
//! matching SQLite's JSON-plus-indexed-columns layout.

use anyhow::{Result, anyhow};
use idb::{Cursor, CursorDirection, KeyRange, ObjectStore, Query};
use js_sys::{Object, Reflect};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsValue;

use crate::ceremony::idb_err;
use crate::guard::guarded;

/// Value-object property holding the opaque serialized DTO (`{ …, data: "<json>" }`).
const DATA_PROP: &str = "data";

/// An out-of-line numeric key (episode id, playback episode_id, autoincrement seq).
/// IndexedDB stores numeric keys as JS numbers; `i64` covers every id/seq we use.
pub fn num_key(n: i64) -> JsValue {
    JsValue::from_f64(n as f64)
}

/// An out-of-line string key (a config suffix, a fixed record name).
pub fn str_key(s: &str) -> JsValue {
    JsValue::from_str(s)
}

/// Serialize `value` to JSON and `put` it under `key` (`None` for an
/// auto-increment store, where the key is generated). The request is awaited, so
/// a quota failure surfaces here.
pub async fn put_json<T: Serialize>(
    store: &ObjectStore,
    key: Option<&JsValue>,
    value: &T,
) -> Result<()> {
    let json = serde_json::to_string(value).map_err(idb_err("serialize"))?;
    let v = JsValue::from_str(&json);
    guarded(store.put(&v, key).map_err(idb_err("put"))?)
        .await
        .map_err(idb_err("put (quota?)"))?;
    Ok(())
}

/// Read and deserialize one record by `key`; `None` when absent or unparseable.
pub async fn get_json<T: DeserializeOwned>(store: &ObjectStore, key: JsValue) -> Result<Option<T>> {
    let value: Option<JsValue> = guarded(store.get(key).map_err(idb_err("get"))?)
        .await
        .map_err(idb_err("get"))?;
    Ok(value
        .and_then(|v| v.as_string())
        .and_then(|s| serde_json::from_str(&s).ok()))
}

/// Read and deserialize every record in the store (corrupt rows are skipped).
pub async fn get_all_json<T: DeserializeOwned>(store: &ObjectStore) -> Result<Vec<T>> {
    let values = guarded(store.get_all(None, None).map_err(idb_err("get_all"))?)
        .await
        .map_err(idb_err("get_all"))?;
    Ok(values
        .into_iter()
        .filter_map(|v| v.as_string())
        .filter_map(|s| serde_json::from_str(&s).ok())
        .collect())
}

/// Read every record paired with its numeric key, in key order (ascending — so an
/// auto-increment store yields insertion order). Used by the outbox drain.
pub async fn get_all_with_keys<T: DeserializeOwned>(store: &ObjectStore) -> Result<Vec<(u64, T)>> {
    let keys = guarded(store.get_all_keys(None, None).map_err(idb_err("keys"))?)
        .await
        .map_err(idb_err("keys"))?;
    let values = guarded(store.get_all(None, None).map_err(idb_err("get_all"))?)
        .await
        .map_err(idb_err("get_all"))?;
    anyhow::ensure!(keys.len() == values.len(), "journal keys and rows disagree");
    keys.into_iter()
        .zip(values)
        .map(|(key, value)| {
            let id = key
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("invalid journal key"))?;
            anyhow::ensure!(
                id.is_finite() && id > 0.0 && id.fract() == 0.0 && id <= 9_007_199_254_740_991.0,
                "invalid journal key"
            );
            let json = value
                .as_string()
                .ok_or_else(|| anyhow::anyhow!("invalid journal payload"))?;
            Ok((id as u64, serde_json::from_str(&json)?))
        })
        .collect()
}

/// Delete one record by `key`.
pub async fn delete(store: &ObjectStore, key: JsValue) -> Result<()> {
    guarded(store.delete(key).map_err(idb_err("delete"))?)
        .await
        .map_err(idb_err("delete"))?;
    Ok(())
}

/// Delete every record in the store.
pub async fn clear_store(store: &ObjectStore) -> Result<()> {
    guarded(store.clear().map_err(idb_err("clear"))?)
        .await
        .map_err(idb_err("clear"))?;
    Ok(())
}

// ── Index-friendly (wrapped) records ────────────────────────────────────────
// A record stored as `{ <indexed scalar props…>, data: "<json>" }` so IndexedDB
// indexes can sort/range over the scalar props while the DTO stays an opaque JSON
// string. Written with `put_episode`, read back via the unwrapping helpers below.

/// Set one own-property on a JS object, mapping the `JsValue` error to `anyhow`
/// (`Reflect::set` can't be tagged by `idb_err`, which needs `Display`).
fn set_prop(obj: &Object, key: &str, val: &JsValue) -> Result<()> {
    Reflect::set(obj, &JsValue::from_str(key), val).map_err(|e| anyhow!("set {key}: {e:?}"))?;
    Ok(())
}

/// Persist one episode as an index-friendly object under the out-of-line numeric `key` (the episode id): two scalar
/// index columns (`scalars` = `(prop, value)` pairs, e.g. `[("pod", podcast_id), ("pub", sort_key)]`) beside the opaque
/// DTO JSON string in `data`. The scalar props feed the store's indexes; a record meant to page by an index MUST carry
/// a present numeric value there (a missing/`null` key drops it from the index entirely).
pub async fn put_episode<T: Serialize>(
    store: &ObjectStore,
    key: &JsValue,
    scalars: &[(&str, f64)],
    value: &T,
) -> Result<()> {
    let json = serde_json::to_string(value).map_err(idb_err("serialize"))?;
    let obj = Object::new();
    for (prop, num) in scalars {
        set_prop(&obj, prop, &JsValue::from_f64(*num))?;
    }
    set_prop(&obj, DATA_PROP, &JsValue::from_str(&json))?;
    guarded(store.put(&obj, Some(key)).map_err(idb_err("put"))?)
        .await
        .map_err(idb_err("put (quota?)"))?;
    Ok(())
}

/// Pull + deserialize the opaque `data` JSON string out of a wrapped record;
/// `None` if the property is absent or the JSON is corrupt (skip, don't fail —
/// matching [`get_all_json`]'s tolerance).
fn unwrap_data<T: DeserializeOwned>(value: &JsValue) -> Option<T> {
    let data = Reflect::get(value, &JsValue::from_str(DATA_PROP)).ok()?;
    serde_json::from_str(&data.as_string()?).ok()
}

/// Read + deserialize every wrapped record in the store (corrupt rows skipped).
/// The in-memory fallback for filtered / non-indexed-order pages + filtered counts.
pub async fn get_all_episodes<T: DeserializeOwned>(store: &ObjectStore) -> Result<Vec<T>> {
    let values = guarded(store.get_all(None, None).map_err(idb_err("get_all"))?)
        .await
        .map_err(idb_err("get_all"))?;
    Ok(values.iter().filter_map(unwrap_data).collect())
}

/// Open a value cursor over `index_name`, returning `None` when the range is empty.
async fn open_index_cursor(
    store: &ObjectStore,
    index_name: &str,
    query: Option<Query>,
    direction: Option<CursorDirection>,
) -> Result<Option<Cursor>> {
    let index = store.index(index_name).map_err(idb_err("index"))?;
    guarded(
        index
            .open_cursor(query, direction)
            .map_err(idb_err("open_cursor"))?,
    )
    .await
    .map_err(idb_err("open_cursor"))
}

/// Skip `offset`, then extract up to `limit` cursor values; corrupt entries still advance. Use raw [`Cursor`] requests
/// through [`guarded`] so cancellation clears handlers; `ManagedCursor` bypasses that protection.
async fn collect_cursor<R>(
    cursor: Option<Cursor>,
    offset: usize,
    limit: Option<usize>,
    extract: impl Fn(&Cursor) -> Option<R>,
) -> Result<Vec<R>> {
    let Some(mut cur) = cursor else {
        return Ok(Vec::new());
    };
    if offset > 0 {
        let Some(advanced) = guarded(
            cur.advance(offset as u32)
                .map_err(idb_err("cursor advance"))?,
        )
        .await
        .map_err(idb_err("cursor advance"))?
        else {
            return Ok(Vec::new());
        };
        cur = advanced;
    }
    let mut out = Vec::new();
    loop {
        if limit.is_some_and(|l| out.len() >= l) {
            break;
        }
        if let Some(item) = extract(&cur) {
            out.push(item);
        }
        let Some(next) = guarded(cur.next(None).map_err(idb_err("cursor next"))?)
            .await
            .map_err(idb_err("cursor next"))?
        else {
            break;
        };
        cur = next;
    }
    Ok(out)
}

/// One page of an indexed store via a cursor: skip `offset`, take up to `limit`, in
/// `descending` (`Prev`) or ascending (`Next`) index order — each record's opaque
/// `data` unwrapped + deserialized. Equal index keys tie-break by primary key
/// (ascending under `Next`, descending under `Prev`), matching the in-memory sort.
pub async fn page_by_index<T: DeserializeOwned>(
    store: &ObjectStore,
    index_name: &str,
    descending: bool,
    offset: usize,
    limit: usize,
) -> Result<Vec<T>> {
    let direction = if descending {
        CursorDirection::Prev
    } else {
        CursorDirection::Next
    };
    let cursor = open_index_cursor(store, index_name, None, Some(direction)).await?;
    collect_cursor(cursor, offset, Some(limit), |c| {
        c.value().ok().as_ref().and_then(unwrap_data)
    })
    .await
}

/// Every record whose index key equals `key_eq` (an equality range), value
/// unwrapped + deserialized — one bulk `getAll` request (not a per-record cursor
/// walk), so a podcast's whole episode slice comes back in a single round-trip.
/// Drives an equality lookup (e.g. all episodes of one podcast via `idx_pod`).
pub async fn scan_index_eq<T: DeserializeOwned>(
    store: &ObjectStore,
    index_name: &str,
    key_eq: f64,
) -> Result<Vec<T>> {
    let index = store.index(index_name).map_err(idb_err("index"))?;
    let range = KeyRange::only(&JsValue::from_f64(key_eq)).map_err(idb_err("key range"))?;
    let values = guarded(
        index
            .get_all(Some(range.into()), None)
            .map_err(idb_err("index get_all"))?,
    )
    .await
    .map_err(idb_err("index get_all"))?;
    Ok(values.iter().filter_map(unwrap_data).collect())
}

/// The out-of-line primary keys (record ids) whose index key equals `key_eq`,
/// without deserializing the values — one bulk `getAllKeys` request. (`getAllKeys`
/// on an index yields the records' primary keys.) For id-only prune lookups.
pub async fn scan_index_eq_ids(
    store: &ObjectStore,
    index_name: &str,
    key_eq: f64,
) -> Result<Vec<i64>> {
    let index = store.index(index_name).map_err(idb_err("index"))?;
    let range = KeyRange::only(&JsValue::from_f64(key_eq)).map_err(idb_err("key range"))?;
    let keys = guarded(
        index
            .get_all_keys(Some(range.into()), None)
            .map_err(idb_err("index get_all_keys"))?,
    )
    .await
    .map_err(idb_err("index get_all_keys"))?;
    Ok(keys
        .iter()
        .filter_map(|k| k.as_f64())
        .map(|f| f as i64)
        .collect())
}
