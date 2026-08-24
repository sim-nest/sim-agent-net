//! Immutable-enough identities for model products observed through provider seats.
//!
//! [`CandidateRevision::from_provider`] consumes the delivered provider cards;
//! it does not discover, authenticate, or open providers. A SIM-side trial
//! harness remains a separate study coordinate and is intentionally absent
//! from candidate identity.

mod candidate;
mod census;
mod economics;
mod executor;
mod identity;
mod privacy;
mod protocol;

pub use candidate::{
    ArtifactEvidence, CandidateRevision, IdentityConfidence, ModelLimits, ModelRoute,
    RouteSemantics,
};
pub use census::{CandidateCensus, CandidatePresence, CandidateRecord, IdentityObservation};
pub use economics::*;
pub use executor::*;
pub use identity::{IdentityMismatch, IdentityVerification, verify_observed_identity};
pub use privacy::{OfflineSnapshot, SnapshotError};
pub use protocol::*;

#[cfg(test)]
mod protocol_tests;
#[cfg(test)]
mod tests;
