# Verify locally

Use injected JWK bytes and an explicit algorithm allowlist; never infer an
algorithm from a token or retrieve keys in the verifier.

This is a sandbox descriptor because verification consumes caller-injected JWK
bytes and token material rather than cookbook secrets. Crate tests execute
allowlist, authority, resource, scope, expiry, and rotation checks.
