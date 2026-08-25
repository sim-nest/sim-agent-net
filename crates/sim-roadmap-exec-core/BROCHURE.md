# Replay execution without replaying effects

`sim-roadmap-exec-core` gives roadmap runners and auditors one shared set of
execution laws. It turns observations into deterministic state and explicit
effect requests while keeping authority, adapter handles, and I/O outside the
crate. Strict correlation checks and a forged-success-resistant receipt gate
make reconciliation reusable across local runners, supervisors, and audits.
