# sim-lib-roadmap-runner

In one line: Durable journal adapter for SIM work plan executions.

## What it gives you

One bounded, redacted, canonical record family captures every work plan execution decision. Cause-only semantic deltas avoid copied carry state, persistent evidence-set roots avoid linear reference lists, and verified snapshots bind exact reducer state to the covered head. Replay verifies identity, order, legality, retention roots, and complete object closure without repeating an effect. The same runner loads bounded plan-refiner and implementer conduct as content-pinned data while strict public validators retain authority over grounding, descent, promise coverage, limits, and admission.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, this crate owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
