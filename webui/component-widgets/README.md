Shared presentation components used across views and pages.

- Provides form fields, confirmations, artwork, rich text, menus, and list controls.
- Rich text passes through a restricted HTML representation before rendering.
- Reads list control types from `listview`; resource mutations go through supplied callbacks.
- Episode menu labels and icons are shared with row and bulk actions.

## Destructive confirmations

```rust
let confirm = use_confirm();
// in a menu callback:
confirm.purge_episode(episode_id);            // local-only cache prune
confirm.delete_podcast(podcast_id);           // server-side unsubscribe
confirm.delete_podcast_then(id, on_deleted);  // then navigate away
confirm.delete_playlist(playlist_id);         // server-side playlist delete
```

## Modal shell

```rust
if confirm_restart() {
    ConfirmModal {
        title: "Restart the server?",
        body: "…",
        confirm_label: "Restart",
        busy: restarting(),
        on_cancel: move |_| confirm_restart.set(false),
        on_confirm: move |_| { /* spawn the action */ },
    }
}
```

## Quick menu

```rust
let quick = use_quick_menu();
quick.open("Episode title".into(), vec![vec![
    QuickAction::new("Add to queue", QuickIcon::Queue, move || { /* … */ }),
]]);
```
