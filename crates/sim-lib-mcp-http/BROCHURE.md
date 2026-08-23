# Streamable HTTP without hidden sessions

Put the stateless SIM MCP service behind a real HTTP boundary while retaining
the same typed application outcome as direct invocation. The adapter validates
origin and wire authority before dispatch, supports backpressured JSON or SSE,
and cancels exactly the request whose delivery disappears. Client traffic uses
SIM's bounded, policy-complete HTTP organ with credentials marked sensitive.
