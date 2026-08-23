//! Compatibility names for the shared platform-time contract.

pub use sim_host_core::{
    DeterministicTime as DeterministicWallClock, SystemWallClock, WallClock, WallTimestamp,
};

#[deprecated(since = "0.1.7", note = "use sim_host_core::WallClock")]
pub use sim_host_core::WallClock as GatewayClock;

#[deprecated(since = "0.1.7", note = "use sim_host_core::SystemWallClock")]
pub use sim_host_core::SystemWallClock as SystemGatewayClock;

#[deprecated(since = "0.1.7", note = "use sim_host_core::DeterministicTime")]
pub use sim_host_core::DeterministicTime as DeterministicGatewayClock;
