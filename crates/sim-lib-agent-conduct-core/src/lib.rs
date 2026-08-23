//! Pure, codec-stable data contracts for agent conduct.
//!
//! This crate contains no execution surface. Its Citizens can be inspected,
//! shaped, encoded, journaled, and validated without loading an agent runtime.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod journal;
mod records;
mod sha256;
pub mod symbols;
mod usage;

pub use journal::{AgentJournal, JournalError};
pub use records::{
    AgentConductContract, AgentEvent, AgentJournalHead, AgentJournalRecord, AgentOutcome,
    AgentRunFrame, AgentRunState, AgentStepCard, AgentStop,
};
pub use usage::{AgentUsage, AgentUsageBudget, UsageError, UsageQuantity};

/// Registers every public conduct record and its generated constructor Shape.
pub fn register_citizens(registry: &mut sim_citizen::CitizenRegistry) -> sim_kernel::Result<()> {
    registry.register::<AgentRunState>()?;
    registry.register::<AgentOutcome>()?;
    registry.register::<AgentStop>()?;
    registry.register::<AgentUsageBudget>()?;
    registry.register::<AgentUsage>()?;
    registry.register::<AgentEvent>()?;
    registry.register::<AgentJournalHead>()?;
    registry.register::<AgentJournalRecord>()?;
    registry.register::<AgentStepCard>()?;
    registry.register::<AgentConductContract>()?;
    registry.register::<AgentRunFrame>()?;
    Ok(())
}

#[cfg(test)]
mod tests;
