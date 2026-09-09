# halogen-router

Application route registration and in-process dispatch.

- build_router composes the remote HTTP application; router_local binds a trusted local actor.
- dispatch sends structured requests through Tower without opening a socket.
- The local route set excludes remote account and process administration surfaces.
