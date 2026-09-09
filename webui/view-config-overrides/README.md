Routed config overrides form UI.

- Routes: `/settings/config/overrides`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
- Modules: `page`, `params`.
