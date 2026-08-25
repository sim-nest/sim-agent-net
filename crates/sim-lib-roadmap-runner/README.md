# sim-lib-roadmap-runner

`sim-lib-roadmap-runner` binds roadmap execution facts to the generic,
domain-free `sim-lib-journal`. It owns the versioned execution payload codec,
redaction and byte budgets, legal replay projection, and immutable identity
branching. The storage crate remains the sole owner of objects, atomic fenced
append, hash-chain verification, durable backends, and crash recovery.

Call `ExecutionJournal::open` with every pinned identity. Append records using
the returned expected head. A stale writer must replay. `rebuild` is read-only
and performs no execution effects.

For phase proof, admit the grounded phase's complete `ProofCatalog` and allow a
conduct package to select only a leaf name. Command leaves bind literal argv,
an allowlisted sealed environment, opaque read-only source and optional scratch
mounts, complete required sandbox controls, resource bounds, and a structured
stdout expectation. `TypedProofReceipt` keeps operational exit separate from
semantic success and normalizes stops, signals, truncation, launcher and sandbox
identity, output content refs, and the supplied observation timestamp. Pure
artifact-equality and source-deck predicate leaves never launch. Use
`execute_journaled_proof` with a durable `ProofReceiptStore`: it appends effect
intent before launch, persists the launcher receipt before journal completion,
and reconciles any unresolved intent without dispatching it again.

For execution source truth, construct `SourceDeckProvider` with the boot-selected
`SandboxLauncher`, a read-only `SourceRepository`, the published repo-contract
decoder, and the Index fragment decoder. `provide` runs only the request's
literal argv with a sealed environment, a read-only source mount, required
network isolation, timeout, and output cap. It verifies the exact checkout head,
artifact identity, allowed relative paths, query coverage, and source bytes
before `sim-source-deck` mints the deck identity. Keep the returned artifact and
deck receipts together; `SourceDeckReceipt::reusable_after` rejects reuse when a
touched source or generated artifact intersects the receipt dependency set.
