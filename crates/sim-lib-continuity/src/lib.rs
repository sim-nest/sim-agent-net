//! Pure continuity planning and replay.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codec;
mod journal;
mod model;
mod plan;
mod reduce;

pub use codec::{CURRENT_SCHEMA_VERSION, MigrationError, migrate};
pub use journal::{ContinuityJournal, JournalError, JournalRow, MemoryJournal};
pub use model::{ContinuityEvent, ContinuityIntent, ContinuityRefusal, ContinuityTurn, RouteLease};
pub use plan::{ContinuityPlan, NetworkPolicy, RoleDemand};
pub use reduce::{ContinuityState, apply, rebuild};

/// Registers all public continuity Citizens and constructor Shapes.
pub fn register_citizens(registry: &mut sim_citizen::CitizenRegistry) -> sim_kernel::Result<()> {
    registry.register::<RoleDemand>()?;
    registry.register::<RouteLease>()?;
    registry.register::<ContinuityPlan>()?;
    registry.register::<ContinuityEvent>()?;
    registry.register::<ContinuityIntent>()?;
    registry.register::<ContinuityTurn>()?;
    registry.register::<ContinuityRefusal>()?;
    Ok(())
}

#[cfg(test)]
mod tests;
