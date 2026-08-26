# sim-lib-mcp-legacy

In one line: Initialize-era MCP compatibility adapter for SIM.

## What it gives you

The adapter retains initialize-era negotiation and shutdown facts, then constructs a complete immutable request context and calls the canonical stateless MCP service. compatibility behavior is isolated and removable. Modern and compatibility clients share the same service dispatch and validation path. Connection state cannot leak into the reusable service object. Use this package at an old MCP connection boundary. New integrations depend directly on `sim-lib-mcp`. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, this crate owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
