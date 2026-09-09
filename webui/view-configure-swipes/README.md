Routed configure swipes UI.

- Routes: `/settings/configure-swipes`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
