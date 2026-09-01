# sim-lib-oauth-jose

Pure local JWT verification against injected JWK-set documents. Algorithms are
allowlisted and selected independently of key material; `kid`, issuer,
audience/resource, scope, expiry, and skew are checked before identity release.
