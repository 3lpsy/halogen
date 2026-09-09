Routed page login UI.

- Routes: `/auth/login`, `/auth/embedded-setup`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
- Modules: `card`, `error_message`, `login`, `embedded_setup`.
