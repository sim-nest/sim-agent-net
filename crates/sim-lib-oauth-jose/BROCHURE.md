# Local, rotation-aware token verification

`sim-lib-oauth-jose` verifies signed JWT access tokens against bounded injected
JWK sets with explicit algorithms, key ids, rotation generations, and clock
skew. It performs no network or storage I/O and never formats token bytes.
