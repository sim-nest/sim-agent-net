//! Compatibility path for the shared platform-time contracts.

pub use sim_host_core::{
    DeterministicTime as DeterministicWallClock, SystemWallClock, WallClock, WallTimestamp,
};

#[cfg(test)]
mod tests;
