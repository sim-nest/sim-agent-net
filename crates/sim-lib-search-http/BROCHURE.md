# Provider-neutral HTTP search

Turn any safe `SearchWireCodec` into a discoverable Retriever and Tool without
changing agents, CLI, or MCP code. Each configured site has explicit concurrency,
interval, timeout, response, pagination, and egress limits. Live calls and
cassette replay share stable search records, while secrets remain behind an
opaque principal reference and never enter cards, cache keys, captures, or audit.

The fixture recipe demonstrates the extension seam before any provider adapter
is selected.
