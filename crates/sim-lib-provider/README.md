# sim-lib-provider

Reusable setup contracts for provider identity, discovery, and seat opening.

The crate describes open provider families and independently selectable seats.
Adapters enumerate seats and open one as the existing
`sim_lib_agent_runner_core::ModelRunner`; inference, HTTP, subprocess, BRIDGE,
and CLI parsing remain in their owning crates.
