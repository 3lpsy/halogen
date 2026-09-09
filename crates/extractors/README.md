# halogen-extractors

Validated Axum request extraction.

- Id accepts positive i32 path identifiers.
- Body unwraps RequestData; query and body validation produce the shared API error shape.
- Authorization remains in auth and guards.
