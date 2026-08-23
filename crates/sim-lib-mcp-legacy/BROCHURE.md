# sim-lib-mcp-legacy

In one line: Keep older MCP clients working without making their connection lifecycle part of the modern service.

## What it gives you

The adapter retains initialize-era negotiation and shutdown facts, then constructs a complete immutable request context and calls the canonical stateless MCP service.

## Why you will be glad

- Legacy behavior is isolated and removable.
- Modern and legacy clients share the same service dispatch and validation path.
- Connection state cannot leak into the reusable service object.

## Where it fits

Use this package at an old MCP connection boundary. New integrations depend directly on `sim-lib-mcp`.
