# halogen-runtime-control

Graceful process restart coordination.

- RestartHandle records a request and wakes the server shutdown future.
- The server shell drains requests before re-executing with the same arguments and environment.
