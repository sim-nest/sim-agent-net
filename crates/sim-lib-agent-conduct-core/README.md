# sim-lib-agent-conduct-core

Pure Citizen records for describing an agent run, its open lifecycle vocabulary,
domain usage budgets, step Cards, and a canonical hash-linked journal. The
`DurableAgentRun` coordinator appends every completed production step, verifies
resume and receipt replay, reconciles uncertain effects without repeating them,
and creates authority-checked checkpoint forks through an injected journal store.

The crate deliberately cannot execute an agent, model, tool, graph, or host
effect. It depends only on SIM's kernel, value, Shape, and Citizen data layers.
Every public record is a Citizen and therefore has a checked constructor Shape
and the standard general-purpose expression codec round trip.

See `recipes/01-basics/pure-journal` for an entirely local two-record chain.
