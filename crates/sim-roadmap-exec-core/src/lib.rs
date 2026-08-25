#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod card;
mod failure;
mod model;
mod recovery;
mod reduce;

pub use card::*;
pub use failure::*;
pub use model::*;
pub use recovery::*;
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
