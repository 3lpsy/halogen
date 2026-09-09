Storage recovery operations for the `/cache-control` view.

- Works directly against storage when the normal worker cannot start.
- Separates content, audio, queued changes, view settings, logs, and cached web assets.
- Content resets clear sync cursors with metadata; durable queued actions are preserved.
- Purges cover account namespaces; native implementations use application data paths.
