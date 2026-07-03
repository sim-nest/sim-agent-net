# Content-addressed fleet replay

This recipe describes three nodes resolving work by content id through
`EvalFabric`. Node B holds the reply for the stable `shared` request. A caller
asks through `ContentAddressedFabric` with `realize`, not through a transport
API; the fabric checks local content, asks the holder, records the immutable
reply locally, and then replays the same request from the local cassette after
node loss. Unknown content fails closed.

Exact commands:

```bash
cargo test -p sim-lib-stream-fabric --test three_node_store
cargo test -p sim-lib-stream-fabric --test store_replay
```
