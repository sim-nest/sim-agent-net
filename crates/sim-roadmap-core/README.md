# sim-roadmap-core

`sim-roadmap-core` defines immutable, bounded roadmap revisions and their
reviewed implementation guides. A roadmap states intent; it does not execute,
schedule, scan source, invoke a model, or decide that work is complete.

Construction is fail-closed. IDs and prose are validated, references and guide
bindings are checked, limits are enforced, then a schema-versioned canonical
`Datum` is hashed. Acceptance statements can be projected as kernel `Claim`
values with exact supporting references, while proof and completion policy stay
explicitly in this domain.

See `examples/two_phase.rs` for a root and leaf whose guide binds one public
anchor, one exact source excerpt, one promise, and a short Rust sketch.
