use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_store() -> CacheDatabase {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("halogen_test_{}_{}.db", std::process::id(), n));
    let _ = std::fs::remove_file(&path);
    CacheDatabase::open(path).unwrap()
}

/// Like [`temp_store`] but keeps the path so a test can reopen the same DB.
fn temp_store_at() -> (CacheDatabase, std::path::PathBuf) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("halogen_test_{}_r{}.db", std::process::id(), n));
    let _ = std::fs::remove_file(&path);
    (CacheDatabase::open(path.clone()).unwrap(), path)
}

/// One corrupt cached row must be SKIPPED, not poison every read of the
/// table (pre-fix a single bad blob failed all podcast/episode lists until
/// a manual wipe; web has always skipped bad rows).
#[test]
fn corrupt_cache_row_is_skipped_not_fatal() {
    let store = temp_store();
    store.upsert_podcasts(&[sample_podcast(1)]).unwrap();
    store
        .conn
        .borrow_mut()
        .execute(
            "INSERT INTO podcasts (id, data) VALUES (999, 'not json')",
            [],
        )
        .unwrap();
    let rows = store.list_podcasts().expect("read survives bad row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 1);
}

/// A cache-shape version bump clears the CACHE tables on reopen but keeps
/// the durable outbox — the native mirror of the web `SCHEMA_VERSION`
/// Cache-drop semantics.
#[test]
fn stale_cache_version_wipes_cache_but_keeps_outbox() {
    let (store, path) = temp_store_at();
    store.upsert_podcasts(&[sample_podcast(1)]).unwrap();
    store
        .enqueue(&OutboxOp::MarkPlayed {
            episode_id: 7,
            played: true,
        })
        .unwrap();
    // Simulate a DB written by an older cache shape.
    store
        .conn
        .borrow_mut()
        .pragma_update(None, "user_version", CACHE_SCHEMA_VERSION - 1)
        .unwrap();
    drop(store);

    let store = CacheDatabase::open(path).unwrap();
    assert!(
        store.list_podcasts().unwrap().is_empty(),
        "stale cache cleared"
    );
    assert_eq!(
        store.pending().unwrap().len(),
        1,
        "durable outbox preserved"
    );
}

fn sample_podcast(id: i32) -> PodcastData {
    let now = chrono::Utc::now();
    PodcastData {
        id,
        title: format!("Podcast {id}"),
        description: String::new(),
        feed_url: format!("https://example.com/{id}.xml"),
        art_url: None,
        art_file_path: None,
        author: None,
        etag: None,
        last_modified: None,
        polled_at: None,
        podcast_config_id: None,
        podcast_config: None,
        created_at: now,
        updated_at: now,
        episode_count: None,
        feed_url_redirects: None,
    }
}

fn sample_episode(id: i32, podcast_id: i32) -> EpisodeData {
    let now = chrono::Utc::now();
    EpisodeData {
        id,
        podcast_id,
        title: format!("Episode {id}"),
        description: Some("desc".into()),
        content_url: "https://example.com/a.mp3".into(),
        guid: None,
        art_url: None,
        published_at: Some(now),
        downloaded_at: None,
        content_file_path: None,
        download_size: None,
        art_file_path: None,
        download_status: halogen_wire::DownloadStatus::NotDownloaded,
        download_started_at: None,
        download_attempts: 0,
        playback_status: halogen_wire::PlaybackStatus::Unplayed,
        duration_secs: Some(123),
        created_at: now,
        updated_at: now,
        podcast: None,
        playback: None,
        chapters: None,
    }
}

#[test]
fn podcasts_episodes_playbacks_roundtrip() {
    let store = temp_store();

    store
        .upsert_podcasts(&[sample_podcast(1), sample_podcast(2)])
        .unwrap();
    let mut got = store.list_podcasts().unwrap();
    got.sort_by_key(|p| p.id);
    assert_eq!(got.len(), 2);
    assert_eq!(got[1].feed_url, "https://example.com/2.xml");

    // Upsert replaces (no duplicate) and round-trips duration_secs.
    store
        .upsert_episodes(&[
            sample_episode(10, 1),
            sample_episode(11, 1),
            sample_episode(20, 2),
        ])
        .unwrap();
    store.upsert_episodes(&[sample_episode(10, 1)]).unwrap();
    let eps_p1 = store.list_episodes(1).unwrap();
    assert_eq!(
        eps_p1.len(),
        2,
        "episodes filtered by podcast, deduped by id"
    );
    assert_eq!(eps_p1[0].duration_secs, Some(123));
    assert_eq!(store.list_episodes(2).unwrap().len(), 1);

    let pb = PlaybackData {
        id: 1,
        user_id: 1,
        episode_id: 10,
        cursor: 42,
        completed: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    store.save_playback(&pb).unwrap();
    let mut pb2 = pb.clone();
    pb2.cursor = 99;
    store.save_playback(&pb2).unwrap();
    let pbs = store.list_playbacks().unwrap();
    assert_eq!(pbs.len(), 1, "playback upserts on episode_id");
    assert_eq!(pbs[0].cursor, 99);
}

#[test]
fn list_episodes_page_orders_and_offsets() {
    use chrono::{TimeZone, Utc};

    let store = temp_store();
    // Three podcasts' episodes interleaved; distinct publish years so the
    // expected newest-first order spans podcasts (cross-podcast paging).
    let mut eps = Vec::new();
    for (id, year) in [(10, 2020), (11, 2023), (20, 2021), (21, 2022)] {
        let mut e = sample_episode(id, if id < 20 { 1 } else { 2 });
        e.published_at = Some(Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap());
        eps.push(e);
    }
    store.upsert_episodes(&eps).unwrap();

    let q = |page| EpisodeQuery {
        order_by: EpisodeOrder::PublishedAt,
        descending: true,
        page,
        size: 2,
        filter: EpisodeQueryFilter::default(),
    };
    // Page 0: two newest (2023, 2022); page 1: next two (2021, 2020).
    let p0 = store.list_episodes_page(&q(0)).unwrap();
    assert_eq!(p0.iter().map(|e| e.id).collect::<Vec<_>>(), vec![11, 21]);
    let p1 = store.list_episodes_page(&q(1)).unwrap();
    assert_eq!(p1.iter().map(|e| e.id).collect::<Vec<_>>(), vec![20, 10]);
    // Past the end → empty, signalling no more pages.
    assert!(store.list_episodes_page(&q(2)).unwrap().is_empty());
}

#[test]
fn list_episodes_page_title_order_uses_in_memory_fallback() {
    let store = temp_store();
    // Titles out of id order so a pure id sort would fail; only the Title
    // fallback path (non-indexed column) yields alphabetical.
    let mut eps = Vec::new();
    for (id, title) in [(10, "Charlie"), (11, "Alpha"), (12, "Bravo")] {
        let mut e = sample_episode(id, 1);
        e.title = title.to_string();
        eps.push(e);
    }
    store.upsert_episodes(&eps).unwrap();

    let page = store
        .list_episodes_page(&EpisodeQuery {
            order_by: EpisodeOrder::Title,
            descending: false,
            page: 0,
            size: 10,
            filter: EpisodeQueryFilter::default(),
        })
        .unwrap();
    assert_eq!(
        page.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
        vec!["Alpha", "Bravo", "Charlie"],
    );
}

#[test]
fn playlists_and_episodes_by_ids_roundtrip() {
    let store = temp_store();
    store
        .upsert_episodes(&[
            sample_episode(10, 1),
            sample_episode(11, 1),
            sample_episode(12, 1),
        ])
        .unwrap();

    let now = chrono::Utc::now();
    let pl = PlaylistData {
        id: 5,
        name: "PL".into(),
        description: None,
        is_default: false,
        position: 0,
        on_remove_delete_file_server: false,
        on_remove_delete_file_client: false,
        created_at: now,
        updated_at: now,
        episode_ids: Some(vec![12, 10]),
        episode_playlist: None,
    };
    store.upsert_playlists(&[pl]).unwrap();

    let got = store.list_playlists().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].episode_ids,
        Some(vec![12, 10]),
        "ordered ids persist"
    );

    // Resolve bodies by id, ignoring misses (999 absent).
    let mut ids: Vec<i32> = store
        .episodes_by_ids(&[12, 10, 999])
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec![10, 12]);
}

#[test]
fn outbox_fifo_enqueue_pending_ack() {
    let store = temp_store();
    store
        .enqueue(&OutboxOp::MarkPlayed {
            episode_id: 1,
            played: true,
        })
        .unwrap();
    store
        .enqueue(&OutboxOp::SetCursor {
            episode_id: 2,
            cursor: 5,
        })
        .unwrap();

    let pending = store.pending().unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending[0].0 < pending[1].0, "outbox should be FIFO by id");

    let first = pending[0].0;
    store.ack(first).unwrap();
    let pending = store.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0].0, first);
}

/// Playback row for `episode_id` (cursor unused by the prune tests).
fn sample_playback(id: i32, episode_id: i32) -> PlaybackData {
    let now = chrono::Utc::now();
    PlaybackData {
        id,
        user_id: 1,
        episode_id,
        cursor: 0,
        completed: false,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn delete_episode_removes_row_and_playback() {
    let store = temp_store();
    store
        .upsert_episodes(&[sample_episode(10, 1), sample_episode(11, 1)])
        .unwrap();
    store.save_playback(&sample_playback(1, 10)).unwrap();
    store.save_playback(&sample_playback(2, 11)).unwrap();

    store.delete_episode(10).unwrap();

    // The targeted episode + its playback are gone; the sibling survives.
    let eps: Vec<i32> = store
        .list_episodes(1)
        .unwrap()
        .iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(eps, vec![11]);
    let pb_eps: Vec<i32> = store
        .list_playbacks()
        .unwrap()
        .iter()
        .map(|p| p.episode_id)
        .collect();
    assert_eq!(pb_eps, vec![11]);
}

#[test]
fn delete_podcast_removes_podcast_episodes_and_playbacks() {
    let store = temp_store();
    store
        .upsert_podcasts(&[sample_podcast(1), sample_podcast(2)])
        .unwrap();
    store
        .upsert_episodes(&[
            sample_episode(10, 1),
            sample_episode(11, 1),
            sample_episode(20, 2),
        ])
        .unwrap();
    store.save_playback(&sample_playback(1, 10)).unwrap();
    store.save_playback(&sample_playback(2, 20)).unwrap();

    store.delete_podcast(1).unwrap();

    // Podcast 1 and all its episodes/playbacks gone; podcast 2 untouched.
    let pods: Vec<i32> = store
        .list_podcasts()
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(pods, vec![2]);
    assert!(store.list_episodes(1).unwrap().is_empty());
    assert_eq!(store.list_episodes(2).unwrap().len(), 1);
    let pb_eps: Vec<i32> = store
        .list_playbacks()
        .unwrap()
        .iter()
        .map(|p| p.episode_id)
        .collect();
    assert_eq!(
        pb_eps,
        vec![20],
        "only the surviving podcast's playback remains"
    );
}

#[test]
fn clear_empties_store() {
    let store = temp_store();
    store.upsert_podcasts(&[sample_podcast(1)]).unwrap();
    store.upsert_episodes(&[sample_episode(10, 1)]).unwrap();
    store
        .enqueue(&OutboxOp::Subscribe {
            feed_url: "https://example.com/feed".into(),
            title: None,
            description: None,
            author: None,
        })
        .unwrap();
    store.clear().unwrap();
    assert!(store.pending().unwrap().is_empty());
    assert!(store.list_podcasts().unwrap().is_empty());
    assert!(store.list_episodes(1).unwrap().is_empty());
}

#[test]
fn a_failed_journal_insert_rolls_back_the_cached_change() {
    let store = temp_store();
    store.conn.borrow_mut().execute_batch("CREATE TRIGGER reject_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'full'); END;").unwrap();
    let changes = halogen_sync_enrich::StoreChanges {
        playbacks: vec![sample_playback(1, 42)],
        ..Default::default()
    };
    assert!(
        store
            .commit_changes(
                &changes,
                &[OutboxOp::SetCursor {
                    episode_id: 42,
                    cursor: 18
                }]
            )
            .is_err()
    );
    assert!(store.list_playbacks().unwrap().is_empty());
    assert!(store.pending().unwrap().is_empty());
}

#[test]
fn legacy_import_receipt_and_quarantine_survive_reopen() {
    let (store, path) = temp_store_at();
    let operations = vec![(
        "legacy-device-operation".into(),
        OutboxOp::SetCursor {
            episode_id: 42,
            cursor: 18,
        },
    )];
    store.import_operations(&operations).unwrap();
    let (id, mut entry) = store.journal_entries().unwrap().remove(0);
    entry.attempts = 10;
    entry.rejection = Some("server rejected this change".into());
    store.save_journal_entry(id, &entry).unwrap();
    drop(store);
    let store = CacheDatabase::open(path).unwrap();
    store.import_operations(&operations).unwrap();
    assert!(store.pending().unwrap().is_empty());
    let entries = store.journal_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.attempts, 10);
    assert_eq!(
        entries[0].1.source_id.as_deref(),
        Some("legacy-device-operation")
    );
    assert!(entries[0].1.rejection.is_some());
}

#[test]
fn imported_cursor_saves_coalesce_without_replaying_old_receipts() {
    let store = temp_store();
    let first = (
        "cursor-1".into(),
        OutboxOp::SetCursor {
            episode_id: 9,
            cursor: 70,
        },
    );
    let second = (
        "cursor-2".into(),
        OutboxOp::SetCursor {
            episode_id: 9,
            cursor: 20,
        },
    );
    store
        .import_operations(&[first.clone(), second.clone()])
        .unwrap();
    let entries = store.journal_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1.operation, second.1);
    store.ack(entries[0].0).unwrap();
    store.import_operations(&[first, second]).unwrap();
    assert!(store.pending().unwrap().is_empty());
}

#[test]
fn concurrent_pull_cannot_replace_a_newer_cursor_or_queued_edit() {
    use halogen_sync_enrich::StoreChanges;
    let (first, path) = temp_store_at();
    let second = CacheDatabase::open(path).unwrap();
    first
        .commit_changes(
            &StoreChanges {
                sync_cursor: Some("epoch:10".into()),
                playbacks: vec![sample_playback(1, 42)],
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    let original = first.list_playbacks().unwrap().remove(0);
    let mut newer = original.clone();
    newer.cursor = 75;
    second
        .commit_changes(
            &StoreChanges {
                check_sync_cursor: true,
                expected_sync_cursor: Some("epoch:10".into()),
                sync_cursor: Some("epoch:20".into()),
                playbacks: vec![newer],
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    assert!(
        first
            .commit_changes(
                &StoreChanges {
                    check_sync_cursor: true,
                    expected_sync_cursor: Some("epoch:10".into()),
                    sync_cursor: Some("epoch:15".into()),
                    playbacks: vec![original.clone()],
                    ..Default::default()
                },
                &[]
            )
            .is_err()
    );
    assert_eq!(first.sync_cursor().unwrap().as_deref(), Some("epoch:20"));
    assert_eq!(first.list_playbacks().unwrap()[0].cursor, 75);

    second
        .enqueue(&OutboxOp::SetCursor {
            episode_id: 42,
            cursor: 20,
        })
        .unwrap();
    assert!(
        first
            .commit_changes(
                &StoreChanges {
                    require_empty_pending: true,
                    check_sync_cursor: true,
                    expected_sync_cursor: Some("epoch:20".into()),
                    sync_cursor: Some("epoch:30".into()),
                    playbacks: vec![original],
                    ..Default::default()
                },
                &[]
            )
            .is_err()
    );
    assert_eq!(first.sync_cursor().unwrap().as_deref(), Some("epoch:20"));
    assert_eq!(first.list_playbacks().unwrap()[0].cursor, 75);
    assert_eq!(first.pending().unwrap().len(), 1);
}
