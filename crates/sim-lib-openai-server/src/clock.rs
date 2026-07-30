//! Compatibility names for the canonical server wall-clock contract.

pub use sim_lib_server::{DeterministicWallClock, SystemWallClock, WallClock, WallTimestamp};

#[deprecated(
    since = "0.1.7",
    note = "use sim_lib_server::WallClock; this compatibility name is removed in sim-lib-openai-server 0.2.0"
)]
pub use sim_lib_server::WallClock as GatewayClock;

#[deprecated(
    since = "0.1.7",
    note = "use sim_lib_server::SystemWallClock; this compatibility name is removed in sim-lib-openai-server 0.2.0"
)]
pub use sim_lib_server::SystemWallClock as SystemGatewayClock;

#[deprecated(
    since = "0.1.7",
    note = "use sim_lib_server::DeterministicWallClock; this compatibility name is removed in sim-lib-openai-server 0.2.0"
)]
pub use sim_lib_server::DeterministicWallClock as DeterministicGatewayClock;
