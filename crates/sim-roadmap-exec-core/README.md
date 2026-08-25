# sim-roadmap-exec-core

`sim-roadmap-exec-core` is the effect-free value machine for executing one
admitted roadmap phase. Hosts submit observations and receive immutable state
transitions plus effect requests as data. The crate never performs filesystem,
process, model, network, Git, journal, scheduling, or CLI work.

The reducer binds every event to an execution, phase, journal head, attempt,
mutation plan, and proof cursor. Success is admitted only after all postimages
are committed, the source deck is current, every required promise is proven,
parent acceptance remains retained, and no mandatory proof is unresolved.

The `ExecutionValueFace` citizen is the canonical open projection for execution
values. Its derived Card, Shape, semantic codec, and versioned read-construct
surfaces are tested through registry conformance; the strongly typed records
remain the reducer API.

## Minimal replay

Build an `ExecutionPolicy`, a canonical `MutationPlan`, and an initial
`Transition`, then feed journal observations to `replay`. A host interprets the
returned `EffectRequest` values and submits the resulting observations in a
later call. Replaying the same ordered event slice against the same initial
value always produces the same result and never repeats an effect itself.
