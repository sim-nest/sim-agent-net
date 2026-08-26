# sim-lib-mcp-client

In one line: Modern-first, transport-neutral MCP client for SIM.

## What it gives you

Start modern, fall back safely before application traffic, and project HTTP or stdio peers into the same SIM runtime objects. `sim-lib-mcp-client` brings validated discovery, Card import, one callable path, bounded MRTR, checked subscriptions, cancellation, deadlines, ledger events, and principal-scoped caching without duplicating transports or credentials. It is deliberately boring where security matters: no speculative replay, no icon fetches, no cache of partial or effecting results, and no ambient input or persistent-storage authority. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-mcp-client owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
