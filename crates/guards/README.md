# halogen-guards

Shared ownership checks for application resources.

- Use ownership guards before reading or mutating actor-scoped records.
- Handlers and routes share these checks; local dispatch does not bypass ownership.
