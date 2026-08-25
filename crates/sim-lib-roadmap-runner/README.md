# sim-lib-roadmap-runner

`sim-lib-roadmap-runner` binds roadmap execution facts to the generic,
domain-free `sim-lib-journal`. It owns the versioned execution payload codec,
redaction and byte budgets, legal replay projection, and immutable identity
branching. The storage crate remains the sole owner of objects, atomic fenced
append, hash-chain verification, durable backends, and crash recovery.

Call `ExecutionJournal::open` with every pinned identity. Append records using
the returned expected head. A stale writer must replay. `rebuild` is read-only
and performs no execution effects.
