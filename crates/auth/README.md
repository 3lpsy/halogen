# halogen-auth

Remote authentication and trusted local actor resolution.

- Token and cookie modules support HTTP credentials; middleware supplies authenticated claims.
- Local dispatch resolves the selected database actor on every request.
- Deleted profiles and changed administrator privileges take effect without reopening a session.
