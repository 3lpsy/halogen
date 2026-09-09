# halogen-config

Layered runtime configuration.

- Resolution order is defaults, TOML, environment, CLI, then runtime overrides.
- Overrides have an allowlist and cannot replace secrets or binding identity.
- Resolution diagnostics are retained until logging starts.
