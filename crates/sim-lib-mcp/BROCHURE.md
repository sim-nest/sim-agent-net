# sim-lib-mcp

In one line: Library-only MCP surface projection for SIM.

## What it gives you

The Model Context Protocol is a common way for assistants to discover and invoke tools and resources. This crate takes SIM's internal catalog, browse cards, and skill descriptions and presents them through one immutable application service. Each decoded request arrives with a fresh execution context and complete caller, negotiation, and cache facts. The same canonical path performs lookup, argument Shape checking, execution, content validation, and result mapping without retaining connection state. SIM's tools and skills become visible to any assistant that speaks this common protocol. Sensitive internals are filtered out. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-mcp owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
