# halogen-art

On-demand podcast and episode artwork caching.

- Cache fetched images under the configured media root and persist their paths.
- Podcast and episode fallback can cross once, preventing recursive fallback loops.
- Per-resource locks avoid duplicate fetches; a negative cache limits repeated failures.
- Application routes enforce actor access before returning cached media.
