# sim-lib-mcp-legacy

Initialize-era MCP compatibility built as a removable, one-way adapter over the stateless `sim-lib-mcp` service.

## API

Use `LegacyConnection` only for peers that require initialize, initialized, and shutdown connection state.

## Examples

See the runnable `legacy-lifecycle` recipe.

## Design

Each ordinary request receives a newly constructed `RequestContext`; no request state enters the modern service object.
