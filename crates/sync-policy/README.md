# halogen-sync-policy

Shared retry and conflict decisions.

- Classify API failures consistently for browser and native mutation queues.
- Keep transport-independent playback and mutation policy here.
- Execution and persistence belong to sync and sync-store.

## Glossary

- Permanent failure: An error that automatic retries cannot resolve.
- Quarantine: Retained failed work excluded from automatic delivery.
