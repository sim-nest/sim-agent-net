# sim-lib-mcp

In one line: A stateless MCP service that turns SIM's tools and skills into safe protocol operations.

## What it gives you

The Model Context Protocol is a common way for assistants to discover and invoke tools and resources. This crate takes SIM's internal catalog, browse cards, and skill descriptions and presents them through one immutable application service. Each decoded request arrives with a fresh execution context and complete caller, negotiation, and cache facts. The same canonical path performs lookup, argument Shape checking, execution, content validation, and result mapping without retaining connection state.

## Why you will be glad

- SIM's tools and skills become visible to any assistant that speaks this common protocol.
- Sensitive internals are filtered out before anything is shared, keeping private details private.
- You describe your capabilities once in SIM and they appear in a standard, discoverable form.
- One service value can safely handle requests in any order: it forks a fresh context and intersects principal grants with admitted operation needs every time.
- Explicit durable and event providers preserve application state without leaking request diagnostics, ledgers, traces, cancellation, or dynamic power.

## Where it fits

Use this crate as the canonical application boundary for modern MCP. A host decodes transport frames, supplies `RequestContext` plus an explicit host seed, and injects any shared provider catalog at construction. Protected MRTR state is re-authorized on retry; subscriptions own bounded queues and cancellation; cache hints come from the codec registry. Initialize-era peers compose the removable `sim-lib-mcp-legacy` adapter.
