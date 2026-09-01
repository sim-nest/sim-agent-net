# Modern stdio composition

This specimen identifies the modern composition: a bounded `StdioServer`
dispatches every request with complete metadata, a fresh context, and an
independent cancellation lifetime. No initialize message or retained client
identity is required.
