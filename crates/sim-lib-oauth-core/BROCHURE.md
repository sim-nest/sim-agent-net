# Reusable OAuth authority without ambient effects

`sim-lib-oauth-core` validates resource and authorization-server metadata,
builds PKCE S256 authorization requests, checks issuer/state/resource binding,
and returns immutable verified principals. Secrets are opaque and redacted.
