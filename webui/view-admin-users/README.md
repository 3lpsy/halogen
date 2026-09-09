Routed admin users UI.

- Routes: `/admin/users`, `/admin/users/create`, `/admin/users/:id/edit`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
