# sim-lib-mcp-client

`sim-lib-mcp-client` is SIM's modern-first client for final stateless MCP and
the delivered initialize-era compatibility baseline. It composes the existing
HTTP and stdio bindings, imports the canonical `SkillCard`, and probes an
unknown endpoint before the first application operation.

The client has no socket, HTTP parser, subprocess launcher, OAuth flow, or
parallel card type. Hosts inject a binding peer, authenticated principal/cache
scope, input broker, ledger, clock facts, cancellation, and an optional
persistent cache whose encryption and privacy policy remain host-owned.

Icon fields are bounded descriptors only. This crate never fetches, decodes,
caches, or renders icon content, and therefore never forwards MCP or OAuth
credentials to icon URIs.

See the recipes for HTTP, stdio, legacy discovery, MRTR, and subscription
composition.
