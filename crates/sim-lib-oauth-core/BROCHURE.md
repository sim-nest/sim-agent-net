# sim-lib-oauth-core

In one line: Effect-free OAuth 2.1 resource and client state machines for SIM.

## What it gives you

`sim-lib-oauth-core` validates resource and authorization-server metadata, builds PKCE S256 authorization requests, checks issuer/state/resource binding, and returns immutable verified principals. Secrets are opaque and redacted. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-oauth-core owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
