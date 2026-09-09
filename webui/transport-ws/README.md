Connectivity transport used by the sync worker.

- Mints connection tickets, opens the socket, exchanges application pings, and reconnects with backoff.
- Browser and native transports share the driver and connection-event vocabulary.
- Reports reachability and latency through events; it never writes UI metadata state.
