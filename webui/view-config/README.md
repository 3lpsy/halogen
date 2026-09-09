Routed view config UI.

- Routes: `/settings/config`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
