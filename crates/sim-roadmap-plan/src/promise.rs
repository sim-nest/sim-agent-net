use sim_roadmap_core::{PhaseId, Promise};
use sim_source_deck::SourceDeck;

/// Read-only verifier for pre-work promise checks.
pub trait PromiseVerifier {
    fn public_declaration_exists(&self, owner: &str, anchor: &str) -> bool;
    fn source_path_is_current(&self, owner: &str, path: &str) -> bool;
    fn specimen_exists(&self, owner: &str, specimen: &str) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromiseState {
    Clear,
    Collision,
    PreconditionsMet,
    Deferred,
}

pub fn check_promise(
    _phase: &PhaseId,
    promise: &Promise,
    deck: &SourceDeck,
    verifier: &dyn PromiseVerifier,
) -> PromiseState {
    match promise {
        Promise::PublicDeclaration { owner, anchor, .. } => {
            if verifier.public_declaration_exists(owner.as_str(), anchor) {
                PromiseState::Collision
            } else {
                PromiseState::Clear
            }
        }
        Promise::SourcePostimage { owner, path, .. } => {
            if deck
                .repositories()
                .iter()
                .any(|r| r.owner == owner.as_str())
                && verifier.source_path_is_current(owner.as_str(), path)
            {
                PromiseState::PreconditionsMet
            } else {
                PromiseState::Deferred
            }
        }
        Promise::CheckedSpecimen {
            owner, specimen, ..
        } => {
            if verifier.specimen_exists(owner.as_str(), specimen) {
                PromiseState::PreconditionsMet
            } else {
                PromiseState::Deferred
            }
        }
        Promise::ProducedOutput { .. } | Promise::Acceptance { .. } => PromiseState::Deferred,
    }
}
