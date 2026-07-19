# Stdio MCP bootloader (descriptor)

Documents the `sim-mcp-server` boot path. The binary starts through the shared
SIM bootloader, selects the MCP codec, and dispatches the MCP serve entrypoint
over stdio. Starting a server is a host-side transport action, so this recipe
records the descriptor contract instead of opening a live channel in the
cookbook sandbox.
