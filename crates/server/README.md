# Server

The network host for the shared podcast application router.

- `hosting` adds CORS and static frontend serving to `halogen-router`.
- `main`, `startup` and `lifecycle` own process startup, migrations, seeding,
  polling, listening and restart.
- `public` serves a development directory; `embedded` serves the prebuilt
  frontend when `embed-frontend` is enabled. Neither builds web assets.
- Application routes, authorization, handlers and queries live in their own crates.
  Native local clients dispatch through `local-runtime` without starting this host.
- `dev-seed` permits test fixtures in a release build and is excluded from shipping builds.

HTTP journeys live in `halogen-integ`; browser journeys use the same host with
its embedded frontend. Run `just test-integ` after changing the hosting boundary.
