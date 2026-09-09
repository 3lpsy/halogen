Device audio storage behind a shared UI service interface.

- Native storage uses files; browser storage uses IndexedDB blobs and object URLs.
- `MediaWriter` stages partial downloads before committing complete audio.
- Metadata and sync journals live separately; stored audio is the source of device-download availability.

## Glossary

- Partial: persisted bytes from an unfinished download.
- Commit: promote staged audio to a playable complete file.
