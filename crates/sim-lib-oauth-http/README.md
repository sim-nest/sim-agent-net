# sim-lib-oauth-http

Bounded retrieval and refresh policy for OAuth discovery and JWK documents.
Actual HTTP execution is injected and must use the returned
`sim-lib-net-http::Policy`; core and JOSE remain I/O-free.
