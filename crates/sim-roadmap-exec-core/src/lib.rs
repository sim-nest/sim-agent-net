#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod model;
mod reduce;

pub use model::*;
pub use reduce::{reduce, replay};

use sim_citizen::CitizenRegistry;
use sim_kernel::Result;

/// Registers every public execution value with the generic SIM citizen surface.
pub fn register_citizens(registry: &mut CitizenRegistry) -> Result<()> {
    registry.register::<ExecutionValueFace>()?;
    Ok(())
}

#[cfg(test)]
mod tests;
