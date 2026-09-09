Per-profile client preferences and persistence.

- Holds connection settings, navigation, swipe actions, playback, downloads, and logging preferences.
- List-view settings use a separate store so list interactions do not rewrite all client settings.
- Builds authenticated API clients and resolves native local-profile transport addresses.
- `AccountKey` and server hashes keep profiles with identical user IDs separate.
