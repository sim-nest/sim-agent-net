# sim-lib-web-fetch

`sim-lib-web-fetch` is the single live landing-page capture owner for SIM. It
composes the existing HTTP membrane, web evidence records, domain codecs,
caller-owned storage, capability checks, robots policy, and injection fences.

Captures are immutable. URL and validator indexes are mutable references to
content-addressed raw and representation records; they never replace evidence.
Every network test and recipe uses an injected transport and performs no DNS or
socket access.
