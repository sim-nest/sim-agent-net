# sim-lib-mcp-http

`sim-lib-mcp-http` is the final-protocol Streamable HTTP boundary for the
stateless `sim-lib-mcp` service. It composes the raw HTTP server seam and the
bounded `sim-lib-net-http` client; it does not own sockets, TLS, an executor,
HTTP parsing, OAuth policy, or session state.

The server exposes one configured POST endpoint. Origin, media negotiation,
body bounds, MCP projection headers, and body/header equality are checked
before application dispatch. Replies are either one JSON document, `202` for
an accepted notification, or a request-scoped SSE sequence. Session headers
are ignored and never emitted. Initialize-era behavior is available only at an
explicitly configured legacy endpoint.
