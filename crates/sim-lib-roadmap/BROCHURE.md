# sim-lib-roadmap

In one line: Bounded native SIM value faces for grounded work plans.

## What it gives you

`sim-lib-work plan` makes work plan reading, grounding, planning, exact comparison, certified refinement, rendering, and bounded explanation ordinary SIM values. Every operation is pure and Shape-checked: callers retain the source bytes and host authority, while the library returns reviewable values or typed refusals. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, this crate owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
