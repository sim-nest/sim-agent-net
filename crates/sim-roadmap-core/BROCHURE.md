# sim-roadmap-core

In one line: Pure content-addressed work plan specifications and implementation guides for SIM.

## What it gives you

`sim-work plan-core` turns authored intent into small, immutable values. Each revision pins its imports, bounds its size, names its acceptance obligations, and gives implementers reviewed guidance tied to exact source evidence. The crate deliberately stops at meaning. Sketches are useful guidance, never proof; acceptance projects into SIM's shared Claim vocabulary without smuggling execution state into the work plan; canonical identities remain stable when unordered authored collections are inserted in a different order. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, this crate owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
