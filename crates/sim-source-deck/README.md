# sim-source-deck

`sim-source-deck` is SIM's pure, portable value for bounded source context. A
caller supplies an already-fetched SIM Index fragment, an exact claim
certificate, and source/specimen bytes. The crate validates their identities and
relations, then mints a deterministic `SourceDeckId` through kernel `Datum`.

The crate performs no filesystem, process, network, parsing, scanning, artifact,
journal, model, or roadmap work. Decode bytes at the boundary with a caller-owned
`FragmentDecoder`; the crate contains no codec implementation. Missing syntax or
scanner coverage is retained as a `Limitation`, never represented as an empty
successful fact.

See `examples/grounded_declaration.rs` for a networkless fixture grounding a
public declaration, a private excerpt, and a test specimen.
