# Resource-bound authorization code

Keep protocol transitions pure while injecting entropy, consent, browser handoff,
and token persistence through explicit interfaces.

This is a sandbox descriptor because a real transition requires caller-owned
entropy, consent, browser handoff, and storage interfaces. Crate tests execute
the pure transition with deterministic injected fixtures.
