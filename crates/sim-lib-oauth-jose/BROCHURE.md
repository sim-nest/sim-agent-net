# sim-lib-oauth-jose

In one line: Local JOSE access-token verification for SIM OAuth.

## What it gives you

`sim-lib-oauth-jose` verifies signed JWT access tokens against bounded injected JWK sets with explicit algorithms, key ids, rotation generations, and clock skew. It performs no network or storage I/O and never formats token bytes. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-oauth-jose owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
