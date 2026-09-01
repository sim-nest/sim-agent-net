# sim-lib-mcp-stdio

`sim-lib-mcp-stdio` is the bounded process-lifetime adapter for MCP over stdin
and stdout. It reads and writes exactly one JSON value per newline, gives every
live inbound id its own cancellation token, preserves complete request `_meta`,
and serializes protocol output through one bounded queue. Diagnostics are sent
only to a caller-provided stderr sink.

The adapter is intentionally not an MCP implementation. Compose it with
`sim-lib-mcp` for modern stateless dispatch, enable `sim-lib-mcp-legacy` only at
process construction for initialize-era peers, and use
`sim-lib-agent-runner-process::ProcessProgram` for client-side child execution.

## Safety contract

- Unterminated, oversize, invalid UTF-8, invalid JSON, multi-value, and embedded
  newline frames fail closed.
- Live ids are unique; cancellation addresses exactly one active token.
- EOF and write failure cancel all still-active work.
- Request execution is independent while protocol writes remain serialized.
- Modern traffic never creates or mutates connection negotiation state.

See `recipes/01-basics/modern-stdio` and `legacy-stdio` for runnable composition
specimens. API details are in rustdoc; package positioning is in `BROCHURE.md`.
