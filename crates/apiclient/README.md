# halogen-apiclient

Typed application gateway for HTTP and native local transport.

- Methods accept wire DTOs and return decoded data or typed ApiError.
- Native LocalTransport dispatches application requests without a listening API server.
- Media streaming and authorized local file access use their own transport methods.
