# halogen-integ

Real HTTP application journeys and their shared harness.

- Tests use a real Axum server, SQLite and the typed ApiClient.
- Only upstream feeds and media are mocked; raw HTTP helpers cover binary and WebSocket protocols.
- Run just test-integ; ordered journeys live in tests/.
