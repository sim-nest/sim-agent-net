# sim-lib-search-http

Provider-neutral HTTP execution for any `SearchWireCodec`. The crate resolves
redacted site configuration through `sim-config`, injects principal headers at
the secret boundary, applies query and per-site bounds, captures raw provider
responses, and exposes the site through ordinary skill cards.

Provider fields, ranking, landing-page retrieval, and credential storage are
deliberately outside this crate.
