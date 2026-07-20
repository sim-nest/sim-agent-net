# Output Contract Repair Recipe

This runnable recipe derives an output contract from a Shape, reads the neutral
grammar graph metadata, selects providers by grammar dialect, and runs a bounded
repair loop against a fake runner.

Run it from the repository root:

```bash
cargo run --manifest-path crates/sim-lib-agent-runner-core/recipes/01-basics/output-contract-repair/Cargo.toml
```

The output reports the graph metadata, the selected providers, and the accepted
response after the fake runner repairs its first answer.
