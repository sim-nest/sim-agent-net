# sim-lib-roadmap-runner

`sim-lib-roadmap-runner` binds roadmap execution facts to the generic,
domain-free `sim-lib-journal`. It owns the versioned execution payload codec,
redaction and byte budgets, legal replay projection, and immutable identity
branching. The storage crate remains the sole owner of objects, atomic fenced
append, hash-chain verification, durable backends, and crash recovery.

Roadmap refinement is loaded behavior, not a runner worker type. Load the
shipped `roadmap/refiner-v1` `RefinerPackage` or a third-party package under the
same proposal/result Shapes. `RefinerFace` exposes only grounded parent and
guide material, the pinned source deck, derived profile, atomicity policy,
remaining bounds, and exact rejection feedback through injection-fenced BRIDGE
fields. `check_proposal_fields` rejects model-authored rank, profile,
certificate, completion, mutation, proof, and authority claims. Pass the typed
`ProposalDraft` to `validate_refinement`; it checks the pinned grounding and
delegates descent, coverage, ceilings, bounds, successor compilation, and the
certificate to `sim-roadmap-refine`. An unanswered source query returns typed
`Blocked` and cannot disappear into an admitted revision.

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

After mutation, form `FreshDeckInvalidation` from every touched path and affected
generated fact, rebuild the repository contract and deck, and call
`admit_fresh_deck`. Bind each `TypedProofReceipt` to a `ProofAuthority` covering
the exact plan, fresh deck, committed mutation, launcher, policy, and proof
definition. `decide_promise` preserves proven, refuted, and inconclusive results;
only a predeclared fallback with remaining budget may resolve an inconclusive
leaf. `accept_all` requires every retained promise plus explicit child evidence
for every parent promise. Finally call `invalidate_readiness` so the compiled
plan rebuilds affected readiness, then offer the typed discharges and fresh deck
to `sim-roadmap-exec-core`; only its reducer may mint `PhaseReceipt`.

For multi-file mutation, first turn every structural edit into exact full
`PortableImage` preimages and postimages, then call `SealedMutationPlan::seal`.
Sealing sorts and deduplicates paths and binds bytes, portable modes, existence,
and ordering into one identity. `MutationEngine` preflights the entire plan
before journaling `Prepared`, applies only sealed postimages through an injected
`MutationWorkspace`, and observes each result before advancing its fence. The
native `FsWorkspace` uses no-follow observation, same-directory temporaries,
file flushes, atomic replacement, and parent-directory synchronization.

Recovery is deliberately conservative: `classify_plan` labels each path as an
exact preimage, postimage, unchanged image, or foreign. A mix of preimages and
postimages resumes deterministically; any foreign path returns `Ambiguous` and
preserves its bytes. `inverse_plan` is an explicit second sealed transaction and
is available only while every relevant path still equals the first plan's
postimage. Adapters report the durability they actually provide in the receipt.

For implementation, load the `roadmap/implementer-v1` topology and render an
`ImplementerFace` from the one grounded leaf, labeled guide examples, bounded
source deck, proof catalog, allowed roots, mutation ceilings, and the prior
typed rejection. The conduct has no effect or tool node. Its receive boundary
accepts only `MutationProposal`, `NeedsRefinement`, or `Blocked`; status prose
has no authority. `admit_implementer_reply` validates guide labels and promise
ids, rejects protected and generated paths, binary content, executable
widening, and out-of-root edits, then delegates exact full-image sealing to the
mutation layer. It returns data only. Read-only inspection is available solely
through exact named `ObservationSpecimen` records with fixed argv, cwd, roots,
network denial, no write mounts, and a hard output ceiling.
