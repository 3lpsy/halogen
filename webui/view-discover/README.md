Discover views hosted by `routes`, separate from library podcast and episode pages.

- `/discover` searches by podcast or episode with provider filters and infinite scrolling.
- `/discover/podcasts/:id` previews a remote feed; `/discover/episodes/:id` shows its episode. `/discover/:id` remains an alias.
- Session state preserves results and scroll position. Refreshing a detail link requires a new search.
- Query, mode, and provider changes invalidate pending results; failed pages retain loaded rows for retry.
- Subscription uses the shared durable command. Descriptions use safe rich text; podcast summaries start collapsed.
