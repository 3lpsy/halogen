# halogen-polling

Scheduled and on-demand feed polling.

- PollingHandle starts, stops and resets the loop; each feed keeps its own polling interval.
- On-demand jobs persist progress through the jobs tracker.
- Each tick also recovers interrupted downloads and applies retention.
