//! Direct browser-storage purges for the `/cache-control` failsafe page.
//!
//! These talk to the browser APIs **directly** (IndexedDB, Cache Storage, the
//! service worker) — never through the sync worker — so they keep working even when
//! the app's data layer is wedged. Every operation is **device-global** (all
//! account namespaces), never per-user.
//!
//! All client data lives in IndexedDB now (see `halogen-ui-idb`): the metadata
//! store `halogen.store.{segment}`, the config store `halogen.config.{segment}`,
//! and the device-global account registry `halogen.accounts`. The segment list is
//! read from the account registry so a clear covers every signed-in account at
//! once; the store/store-name constants come from `halogen_ui_platform::store_keys`
//! (so this crate names the same databases without depending on the producers).

/// Wipe captured device logs: the in-memory ring + the persisted store
/// (IndexedDB `halogen.logs` on web, the log file on native).
pub async fn clear_logs() {
    halogen_ui_logging::clear();
    halogen_ui_logging::store::clear().await;
}

#[cfg(target_arch = "wasm32")]
pub use web::{
    clear_all_storage, clear_audio, clear_cached_assets, clear_content, clear_outbox,
    clear_view_settings, reload_with_message, unregister_service_worker,
};

#[cfg(not(target_arch = "wasm32"))]
pub use native::{
    clear_all_storage, clear_audio, clear_cached_assets, clear_content, clear_outbox,
    clear_view_settings, reload_with_message, unregister_service_worker,
};

#[cfg(target_arch = "wasm32")]
mod web {
    use idb::TransactionMode;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    use halogen_ui_idb::{
        clear_store, commit_tx, delete, get_json, one_store_tx, open_current, str_key,
    };
    use halogen_ui_platform::namespace;
    use halogen_ui_platform::store_keys::{
        ACCOUNTS_DB, ACCOUNTS_KEY, ACCOUNTS_STORE, CONFIG_DB_PREFIX, CONFIG_LIST_VIEWS_KEY,
        CONFIG_STORE, CONTENT_STORES, MEDIA_DB_LEGACY, MEDIA_DB_PREFIX, MEDIA_STORES,
        STORE_DB_PREFIX, STORE_OUTBOX,
    };

    /// Every account's namespace segment: the signed-out `"anon"` plus one `"u{id}"`
    /// per account in the registry. Read directly from `halogen.accounts` so we
    /// don't depend on `halogen-ui-accounts` (which depends on this crate).
    async fn account_segments() -> Vec<String> {
        // Web-only module: embedded accounts (the `e{id}` segments) exist only in
        // native builds, so every web registry entry is a remote `u{id}-{server}`.
        let mut segs = vec![namespace::segment_for(None, false, 0)];
        for (id, server) in account_entries().await {
            segs.push(namespace::segment_for(Some(id), false, server));
        }
        segs
    }

    /// The `(id, server hash)` of each account in the registry (`[]` if
    /// absent/unreadable). Parsed as generic JSON so this crate needn't know the
    /// `Accounts` type. The server hash scopes a remote account's segment, so it's
    /// needed to name the same databases the stores wrote.
    async fn account_entries() -> Vec<(i32, u64)> {
        let Ok(db) = open_current(ACCOUNTS_DB).await else {
            return Vec::new();
        };
        let Ok((tx, store)) = one_store_tx(&db, ACCOUNTS_STORE, TransactionMode::ReadOnly) else {
            return Vec::new();
        };
        let value = get_json::<serde_json::Value>(&store, str_key(ACCOUNTS_KEY))
            .await
            .ok()
            .flatten();
        let _ = tx.await;
        value
            .as_ref()
            .and_then(|v| v.get("users"))
            .and_then(|u| u.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|u| {
                        let id = i32::try_from(u.get("id").and_then(|i| i.as_i64())?).ok()?;
                        let server = u.get("server").and_then(|s| s.as_u64()).unwrap_or(0);
                        Some((id, server))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clear the named object stores in `db_name` (skipping a missing DB/store).
    /// Returns true if at least one store was cleared.
    async fn clear_stores(db_name: &str, stores: &[&str]) -> bool {
        let Ok(db) = open_current(db_name).await else {
            return false;
        };
        let mut any = false;
        for name in stores {
            // A transaction over a non-existent store errors — skip it.
            if let Ok((tx, store)) = one_store_tx(&db, name, TransactionMode::ReadWrite)
                && clear_store(&store).await.is_ok()
                && commit_tx(tx).await.is_ok()
            {
                any = true;
            }
        }
        any
    }

    /// Delete one record by key from `db_name`'s `store_name` (skip if missing).
    async fn delete_record(db_name: &str, store_name: &str, key: &str) -> bool {
        let Ok(db) = open_current(db_name).await else {
            return false;
        };
        let Ok((tx, store)) = one_store_tx(&db, store_name, TransactionMode::ReadWrite) else {
            return false;
        };
        delete(&store, str_key(key)).await.is_ok() && commit_tx(tx).await.is_ok()
    }

    /// Wipe device-downloaded audio across every account's media DB
    /// (`halogen.media.{segment}`) plus the pre-namespacing global store. Talks to
    /// IndexedDB directly (not the worker's connection). `true` if anything cleared.
    pub async fn clear_audio() -> bool {
        let mut any = false;
        for seg in account_segments().await {
            if clear_stores(&format!("{MEDIA_DB_PREFIX}{seg}"), &MEDIA_STORES).await {
                any = true;
            }
        }
        // Orphaned bytes from before per-account namespacing.
        if clear_stores(MEDIA_DB_LEGACY, &MEDIA_STORES).await {
            any = true;
        }
        any
    }

    /// Cached content rows (re-fetchable): podcasts/episodes/playlists/playbacks,
    /// across every account's store DB. Leaves the outbox alone — clearing that
    /// loses unsynced actions. Returns the number of account stores touched.
    pub async fn clear_content() -> usize {
        let mut n = 0;
        for seg in account_segments().await {
            if clear_stores(&format!("{STORE_DB_PREFIX}{seg}"), &CONTENT_STORES).await {
                n += 1;
            }
        }
        n
    }

    /// The offline action queue (the outbox store) across every account's store DB.
    pub async fn clear_outbox() -> usize {
        let mut n = 0;
        for seg in account_segments().await {
            if clear_stores(&format!("{STORE_DB_PREFIX}{seg}"), &[STORE_OUTBOX]).await {
                n += 1;
            }
        }
        n
    }

    /// Per-list sort/filter view preferences (the `list_views` config record),
    /// across every account's config DB.
    pub async fn clear_view_settings() -> usize {
        let mut n = 0;
        for seg in account_segments().await {
            if delete_record(
                &format!("{CONFIG_DB_PREFIX}{seg}"),
                CONFIG_STORE,
                CONFIG_LIST_VIEWS_KEY,
            )
            .await
            {
                n += 1;
            }
        }
        n
    }

    /// Wipe all client data — every account's store + config DB and the
    /// device-global account registry. Effectively a sign-out of every account.
    /// Downloaded audio (`halogen.media`) and device logs (`halogen.logs`) are NOT
    /// touched here — they have their own actions.
    ///
    /// Clears the object stores rather than deleting the databases: a
    /// `deleteDatabase` blocks while any other connection stays open (the worker's
    /// store handle, the config/accounts cached handles), which would hang the
    /// sign-out; clearing is a normal transaction that those open connections don't
    /// block. The page reloads right after, so empty databases are fine.
    pub async fn clear_all_storage() {
        // Read segments from the registry BEFORE clearing it.
        let segs = account_segments().await;
        let store_stores: Vec<&str> = CONTENT_STORES
            .iter()
            .copied()
            .chain(std::iter::once(STORE_OUTBOX))
            .collect();
        for seg in &segs {
            clear_stores(&format!("{STORE_DB_PREFIX}{seg}"), &store_stores).await;
            clear_stores(&format!("{CONFIG_DB_PREFIX}{seg}"), &[CONFIG_STORE]).await;
        }
        clear_stores(ACCOUNTS_DB, &[ACCOUNTS_STORE]).await;
    }

    /// Delete every Cache Storage bucket (the PWA app-shell + JS/WASM/CSS + artwork
    /// caches). Takes effect on the next load.
    pub async fn clear_cached_assets() {
        let Some(win) = web_sys::window() else { return };
        let Ok(caches) = win.caches() else { return };
        let Ok(keys) = JsFuture::from(caches.keys()).await else {
            return;
        };
        for key in js_sys::Array::from(&keys).iter() {
            if let Some(name) = key.as_string() {
                let _ = JsFuture::from(caches.delete(&name)).await;
            }
        }
    }

    /// Unregister the controlling service worker, so the next load installs a fresh
    /// one (paired with [`clear_cached_assets`] this forces brand-new app code).
    pub async fn unregister_service_worker() {
        let Some(win) = web_sys::window() else { return };
        let sw = win.navigator().service_worker();
        // `get_registration()` resolves to the registration (or undefined);
        // `unregister()` is fallible (hence the `Result`).
        let Ok(val) = JsFuture::from(sw.get_registration()).await else {
            return;
        };
        if let Ok(reg) = val.dyn_into::<web_sys::ServiceWorkerRegistration>()
            && let Ok(promise) = reg.unregister()
        {
            let _ = JsFuture::from(promise).await;
        }
    }

    /// Full page reload back to `/cache-control`, carrying `msg` in the `message`
    /// query param so the reloaded page can show it (the live query is stripped by
    /// the router, but startup re-captures it — see `hooks::read_initial_query_param`).
    /// Reloading stays on this page, so it's not a "redirect out".
    pub fn reload_with_message(msg: &str) {
        let enc = JsValue::from(js_sys::encode_uri_component(msg))
            .as_string()
            .unwrap_or_default();
        if let Some(win) = web_sys::window() {
            let _ = win
                .location()
                .set_href(&format!("/cache-control?message={enc}"));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    // Native has no browser IndexedDB / Cache Storage / service worker, so
    // most of these targets are simply absent and no-op. `clear_all_storage`
    // is the exception: its contract is "every signed-in account, tokens and
    // cached data" — on native that means the on-disk account registry and
    // every per-account namespace under the config root. Without it, the
    // failsafe "delete everything" cards would (e.g.) destroy the embedded
    // server's library while leaving a signed-in-looking orphan session
    // behind.
    use halogen_ui_platform::namespace;

    /// The account-segment directories under `root` (`anon` / `e{id}` /
    /// `u{id}-{server}` / legacy `u{id}`), recognized by the canonical
    /// [`namespace::is_account_segment`] — NOT a local pattern: a hand-rolled
    /// matcher here once predated the server-hash segment format and silently
    /// skipped every remote account's data on wipe.
    fn segment_dirs(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(namespace::is_account_segment)
                    && e.path().is_dir()
            })
            .map(|e| e.path())
            .collect()
    }

    /// Wipe device-downloaded audio: every per-account `<data root>/{segment}/audio`
    /// directory plus the pre-namespacing `<data root>/audio`. Direct fs — the
    /// bypass-the-worker failsafe path. `true` if any directory was removed.
    pub async fn clear_audio() -> bool {
        let root = halogen_ui_platform::paths::data_root();
        // The pre-namespacing device-global dir.
        let mut any = std::fs::remove_dir_all(root.join("audio")).is_ok();
        for dir in segment_dirs(&root) {
            let audio = dir.join("audio");
            if audio.is_dir() && std::fs::remove_dir_all(&audio).is_ok() {
                any = true;
            }
        }
        any
    }

    /// Every account-segment store DB under the data root (present files only —
    /// `NativeLocalStore::open` would otherwise CREATE one per segment).
    fn segment_store_dbs() -> Vec<std::path::PathBuf> {
        segment_dirs(&halogen_ui_platform::paths::data_root())
            .into_iter()
            .map(|d| d.join("halogen.db"))
            .filter(|p| p.is_file())
            .collect()
    }

    /// Cached content rows (re-fetchable) across every account's store DB,
    /// leaving each durable outbox alone. Opens the SQLite files directly —
    /// the bypass-the-worker failsafe path (WAL allows a second connection
    /// alongside a live worker). Returns the number of stores touched.
    pub async fn clear_content() -> usize {
        let mut n = 0;
        for db in segment_store_dbs() {
            if halogen_ui_svc_store::NativeLocalStore::open(db)
                .and_then(|s| s.clear_cached_content())
                .is_ok()
            {
                n += 1;
            }
        }
        n
    }

    /// The offline action queue across every account's store DB (direct fs).
    pub async fn clear_outbox() -> usize {
        let mut n = 0;
        for db in segment_store_dbs() {
            if halogen_ui_svc_store::NativeLocalStore::open(db)
                .and_then(|s| s.clear_outbox_rows())
                .is_ok()
            {
                n += 1;
            }
        }
        n
    }

    /// Per-list view preferences: the `list_views.json` file in every account's
    /// config segment (the native `ListViewStore` backend).
    pub async fn clear_view_settings() -> usize {
        let mut n = 0;
        for dir in segment_dirs(&halogen_ui_platform::paths::config_root()) {
            let f = dir.join("list_views.json");
            if f.is_file() && std::fs::remove_file(&f).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Delete `accounts.json` + every account namespace directory under BOTH
    /// the config root (tokens in `client.json`, preferences) and the data
    /// root (`halogen.db` metadata cache + durable outbox, downloaded
    /// `audio/`), plus the pre-namespacing `<data root>/audio`. Direct fs —
    /// this is the bypass-the-worker failsafe path, and its contract is "the
    /// device is clean afterwards": leaving the data root untouched kept every
    /// account's listen history and unsynced actions recoverable on disk after
    /// a "delete everything". On macOS/iOS the two roots coincide — the second
    /// sweep just finds nothing left.
    pub async fn clear_all_storage() {
        let config_root = halogen_ui_platform::paths::config_root();
        let _ = std::fs::remove_file(config_root.join("accounts.json"));
        for dir in segment_dirs(&config_root) {
            let _ = std::fs::remove_dir_all(dir);
        }
        let data_root = halogen_ui_platform::paths::data_root();
        for dir in segment_dirs(&data_root) {
            let _ = std::fs::remove_dir_all(dir);
        }
        // Pre-namespacing device-global audio dir.
        let _ = std::fs::remove_dir_all(data_root.join("audio"));
    }

    pub async fn clear_cached_assets() {}
    pub async fn unregister_service_worker() {}
    pub fn reload_with_message(_msg: &str) {}
}

// Native fs behavior of the failsafe wipes, against real temp directories via
// the `HALOGEN_DATA_DIR`/`HALOGEN_CONFIG_DIR` overrides (safe under nextest's
// process-per-test model; `paths` memoizes per process, so each test process
// must set BOTH env vars before its first `paths` call).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_tests {
    use super::{clear_all_storage, clear_audio, clear_content, clear_outbox, clear_view_settings};

    fn setup_roots(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base =
            std::env::temp_dir().join(format!("halogen-purge-test-{}-{tag}", std::process::id()));
        let data = base.join("data");
        let config = base.join("config");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&data).expect("data root");
        std::fs::create_dir_all(&config).expect("config root");
        unsafe {
            std::env::set_var("HALOGEN_DATA_DIR", &data);
            std::env::set_var("HALOGEN_CONFIG_DIR", &config);
        }
        (data, config)
    }

    fn seed(dir: &std::path::Path, file: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join(file), b"x").expect("seed file");
    }

    #[test]
    fn wipe_covers_every_segment_shape_and_both_roots() {
        let (data, config) = setup_roots("all");
        // The post-H5 remote segment shape the old matcher silently skipped,
        // plus embedded, legacy remote, anon, and non-segment dirs to spare.
        for seg in ["u3-00000000000000ab", "e1", "u2", "anon"] {
            seed(&config.join(seg), "client.json");
            seed(&data.join(seg), "halogen.db");
            seed(&data.join(seg).join("audio"), "1.mp3");
        }
        seed(&data.join("audio"), "legacy.mp3"); // pre-namespacing global dir
        seed(&data.join("server"), "keep.db"); // embedded server library: spared
        std::fs::write(config.join("accounts.json"), b"{}").expect("registry");

        // Seed a real store DB (content + one queued op) and a view-settings
        // file for a post-H5 remote segment: the "clear content" / "clear sync
        // queue" / "clear view settings" cards were silent no-op stubs on
        // native before.
        let seg = "u3-00000000000000ab";
        {
            let store =
                halogen_ui_svc_store::NativeLocalStore::open(data.join(seg).join("halogen.db"))
                    .expect("open store");
            futures::executor::block_on(async {
                use halogen_ui_svc_store::LocalStore;
                store
                    .enqueue(&halogen_ui_svc_store::outbox::OutboxOp::MarkPlayed {
                        episode_id: 1,
                        played: true,
                    })
                    .await
                    .expect("enqueue");
            });
        }
        std::fs::write(config.join(seg).join("list_views.json"), b"{}").expect("views");

        // All four seeded segments carry a halogen.db (the placeholder files
        // are under SQLite's 100-byte threshold, so they open as empty DBs) —
        // every one counts as touched; the real store above is among them.
        assert_eq!(futures::executor::block_on(clear_content()), 4);
        assert_eq!(futures::executor::block_on(clear_outbox()), 4);
        assert_eq!(futures::executor::block_on(clear_view_settings()), 1);
        assert!(!config.join(seg).join("list_views.json").exists());
        {
            let store =
                halogen_ui_svc_store::NativeLocalStore::open(data.join(seg).join("halogen.db"))
                    .expect("reopen store");
            futures::executor::block_on(async {
                use halogen_ui_svc_store::LocalStore;
                assert!(store.pending().await.expect("pending").is_empty());
            });
        }

        assert!(futures::executor::block_on(clear_audio()));
        for seg in ["u3-00000000000000ab", "e1", "u2", "anon"] {
            assert!(
                !data.join(seg).join("audio").exists(),
                "audio not wiped for {seg}"
            );
        }
        assert!(!data.join("audio").exists(), "legacy audio dir not wiped");

        futures::executor::block_on(clear_all_storage());
        for seg in ["u3-00000000000000ab", "e1", "u2", "anon"] {
            assert!(!config.join(seg).exists(), "config segment {seg} survived");
            assert!(!data.join(seg).exists(), "data segment {seg} survived");
        }
        assert!(!config.join("accounts.json").exists(), "registry survived");
        assert!(
            data.join("server").join("keep.db").exists(),
            "non-segment dir must be spared"
        );
    }
}
