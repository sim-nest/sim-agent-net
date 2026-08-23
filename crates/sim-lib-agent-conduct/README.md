# sim-lib-agent-conduct

`sim-lib-agent-conduct` certifies an ordinary `TopologyPackage` against the
`agent/conduct-v1` profile and binds its call nodes to `AgentStepCard`s. It
derives roles and capabilities from the topology and Cards, checks outcome
routing and terminal admission, then delegates compilation, stepping, running,
continuation, and reflection to `sim-lib-topology`.

The crate has no agent facade, runner, BRIDGE, model/provider/tool implementation,
host I/O, registry, graph copy, parser, or scheduler. See the
`recipes/01-basics/echo-conduct` lane for the smallest load-and-run example.
