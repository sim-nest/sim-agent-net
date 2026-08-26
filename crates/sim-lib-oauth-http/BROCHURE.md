# sim-lib-oauth-http

In one line: Bounded OAuth metadata and JWK retrieval over SIM HTTP.

## What it gives you

`sim-lib-oauth-http` makes every metadata/JWK byte cap, redirect decision, refresh interval, and stale-key generation visible. It accepts only HTTPS discovery targets and composes the constellation's one HTTP policy boundary. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-lib-oauth-http owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
