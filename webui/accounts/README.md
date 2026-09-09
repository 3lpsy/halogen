Account registry and profile switching service.

- Stores remote and local profiles in IndexedDB on web and a native JSON file.
- Switching selects the profile's storage namespace and credentials before its providers start.
- Sign-out coordinates account removal and cache cleanup through `account_actions`.
