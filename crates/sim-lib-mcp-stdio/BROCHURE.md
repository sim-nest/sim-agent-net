# MCP stdio without connection authority

Run modern or compatibility MCP peers over real stdio while keeping the
application service stateless. `sim-lib-mcp-stdio` supplies strict bounded JSON
line framing, isolated request cancellation, out-of-order execution, one ordered
writer, stderr-only diagnostics, discovery-aware process clients, and explicit
legacy construction policy.

Use it when a bootloader or process host needs a trustworthy stdio boundary—not
another protocol implementation or a connection-shaped authority cache.
