# sim-lib-mcp-stdio

In one line: Bounded request-lifetime stdio adapter for stateless SIM MCP.

## What it gives you

Run modern or compatibility MCP peers over real stdio while keeping the application service stateless. `sim-lib-mcp-stdio` supplies strict bounded JSON line framing, isolated request cancellation, out-of-order execution, one ordered writer, stderr-only diagnostics, discovery-aware process clients, and explicit compatibility construction policy. Use it when a bootloader or process host needs a trustworthy stdio boundary--not another protocol implementation or a connection-shaped authority cache. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-mcp-stdio owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
