# halogen-net

Outbound request policy and DNS resolution.

- PublicOnlyResolver rejects private and special-purpose addresses when configured.
- The resolver checks new connections, including redirect destinations.
- Startup configures the process-wide policy and optional fetch User-Agent; tests can allow local fixtures.
