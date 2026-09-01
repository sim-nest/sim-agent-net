# sim-lib-mcp-http

In one line: Final-protocol Streamable HTTP adapter for SIM MCP services.

## What it gives you

Put the stateless SIM MCP service behind a real HTTP boundary while retaining the same typed application outcome as direct invocation. The adapter validates origin and wire authority before dispatch, supports backpressured JSON or SSE, and cancels exactly the request whose delivery disappears. Client traffic uses SIM's bounded, policy-complete HTTP organ with credentials marked sensitive. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-mcp-http owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
