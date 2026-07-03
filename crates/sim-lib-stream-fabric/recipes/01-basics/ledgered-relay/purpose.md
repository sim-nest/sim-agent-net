# Ledgered relay descriptor

This recipe describes the content-addressed relay surface. `ContentKey`
identifies a stable eval request, `EvalCassette` stores successful replies by
that key, and `LedgeredRelayFabric` returns cached replies before contacting
the wrapped fabric.

Exact commands:

```bash
cargo test -p sim-lib-stream-fabric ledgered_relay
cargo test -p sim-lib-stream-fabric --test cassette_replay
cargo test -p sim-lib-stream-fabric --test two_node_cadr
```
