use std::rc::Rc;

use futures::{StreamExt, executor::block_on};
use halogen_sync_store::{LocalStore, NativeLocalStore};

use crate::{Command, SyncService, WorkerEvent};

#[test]
fn failed_command_keeps_previous_playback_and_reports_failure() {
    block_on(async {
        let path =
            std::env::temp_dir().join(format!("halogen-worker-rollback-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = Rc::new(NativeLocalStore::open(path.clone()).unwrap());
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let mut service = SyncService::new(store.clone(), None, tx);
        service
            .handle_command(Command::SetCursor {
                episode_id: 7,
                cursor: 10,
            })
            .await;
        let before = (*service.playbacks).clone();
        while rx.try_recv().is_ok() {}
        rusqlite::Connection::open(path).unwrap().execute_batch(
            "CREATE TRIGGER reject_journal BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'disk failure'); END;"
        ).unwrap();
        service
            .handle_command(Command::SetCursor {
                episode_id: 7,
                cursor: 20,
            })
            .await;
        assert_eq!(*service.playbacks, before);
        assert_eq!(store.list_playbacks().await.unwrap()[0].cursor, 10);
        assert!(
            service
                .connection
                .last_error
                .as_deref()
                .unwrap()
                .contains("Could not save action")
        );
        drop(service);
        let events: Vec<_> = rx.collect().await;
        assert!(events.iter().any(|event| matches!(event, WorkerEvent::Toast { message, .. } if message.contains("Couldn't save"))));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, WorkerEvent::Playbacks(_)))
        );
    });
}

#[test]
fn auto_playlist_selection_commits_with_intent_and_survives_reopen() {
    block_on(async {
        let path = std::env::temp_dir().join(format!(
            "halogen-worker-auto-playlists-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let store = Rc::new(NativeLocalStore::open(path.clone()).unwrap());
        let (tx, _) = futures::channel::mpsc::unbounded();
        let mut service = SyncService::new(store.clone(), None, tx);
        service
            .handle_command(Command::SetPodcastAutoPlaylists {
                podcast_id: 1,
                playlist_ids: vec![2, 3],
                add_to_start: Some(true),
            })
            .await;
        rusqlite::Connection::open(&path).unwrap().execute_batch(
            "CREATE TRIGGER reject_journal BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'disk failure'); END;"
        ).unwrap();
        service
            .handle_command(Command::SetPodcastAutoPlaylists {
                podcast_id: 1,
                playlist_ids: vec![4],
                add_to_start: Some(false),
            })
            .await;
        let reopened = NativeLocalStore::open(path).unwrap();
        let selections = reopened.list_auto_playlists().await.unwrap();
        assert_eq!(
            selections[&1]
                .iter()
                .map(|row| row.playlist_id)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert!(
            selections[&1]
                .iter()
                .all(|row| row.add_to_start == Some(true))
        );
        assert_eq!(reopened.pending().await.unwrap().len(), 1);
        assert_eq!(service.podcasts.auto_playlists_by_podcast[&1], vec![2, 3]);
    });
}
