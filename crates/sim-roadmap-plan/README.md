# sim-roadmap-plan

Compiles one admitted roadmap revision against one exact `SourceDeck`. The
result has no scheduling cursor: every readiness query returns the complete
ready set in authored order. Grounding, tractability policy, completion,
outputs, promises, and acceptance are explicit observations, so
`sim-incremental-core` can invalidate only their exact dependents and explain
the bounded causal path.

This crate owns roadmap grounding and transfer policy. It deliberately reuses
`sim-incremental-core` for memoization and reverse dependencies; it contains no
generic frontier, memo table, or reverse-edge walker.
