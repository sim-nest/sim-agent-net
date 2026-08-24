# sim-lib-agent-conduct

`sim-lib-agent-conduct` certifies an ordinary `TopologyPackage` against the
`agent/conduct-v1` profile and binds its call nodes to `AgentStepCard`s. It
derives roles and capabilities from the topology and Cards, checks outcome
routing and terminal admission, then delegates compilation, stepping, running,
continuation, and reflection to `sim-lib-topology`.

The crate has no agent facade, runner, BRIDGE, model/provider/tool implementation,
host I/O, registry, graph copy, parser, or scheduler. See the
`recipes/01-basics/echo-conduct` lane for the smallest load-and-run example.

The `catalog/` directory ships standard agent kinds as data-only topology
packages. `load_agent_conduct_catalog` reads them through table-backed package
sources, registers them with `TopologyRegistry`, and applies the conduct
profile. A new kind is one `.simtopo` package with an embedded test, not a new
Rust loop, type, or function.
The shipped catalog contains default, ReAct, plan-act-replan, phased,
verify-retry, router-crew, and triage packages. The agent CLI and Lisp surfaces
reuse this crate's reflection, diagram, certification, and report APIs.
