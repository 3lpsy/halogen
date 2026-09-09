Playback controller and queue navigation.

- Owns playback transitions and reports progress through typed worker commands.
- Uses `player-types`, `player-backend`, and `player-sleep`; OS controls live in `player-media-session`.
- `ScopeBound` re-enters the Dioxus runtime for browser callbacks.
