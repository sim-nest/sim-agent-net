# sim-lib-search-http

In one line: Provider-neutral, policy-bounded HTTP transport for SIM search codecs.

## What it gives you

Turn any safe `SearchWireCodec` into a discoverable Retriever and Tool without changing agents, CLI, or MCP code. Each configured site has explicit concurrency, interval, timeout, response, pagination, and egress limits. Live calls and cassette replay share stable search records, while secrets remain behind an opaque principal reference and never enter cards, cache keys, captures, or audit. The fixture recipe demonstrates the extension seam before any provider adapter is selected. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-search-http owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
