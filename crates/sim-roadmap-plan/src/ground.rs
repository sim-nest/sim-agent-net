use sim_roadmap_core::{OwnerId, PhaseSpec};
use sim_roadmap_refine::Grounding;
use sim_source_deck::{GroundedEvidence, Limitation, SourceDeck, SourceQuery};

/// Exact evidence and limitations retained for one phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseGrounding {
    pub grounding: Grounding,
    pub evidence: Vec<GroundedEvidence>,
    pub limitations: Vec<Limitation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroundFailure {
    Unresolved(SourceQuery),
    OutOfOwner(SourceQuery),
    Canonical(String),
}

pub fn ground_phase(phase: &PhaseSpec, deck: &SourceDeck) -> Result<PhaseGrounding, GroundFailure> {
    let mut evidence = Vec::new();
    for query in &phase.guide.uses {
        let witness = deck
            .evidence()
            .iter()
            .find(|row| matches_query(row, query))
            .ok_or_else(|| GroundFailure::Unresolved(query.clone()))?;
        let owner = evidence_owner(witness);
        let allowed = phase
            .owners
            .mutable
            .iter()
            .chain(&phase.owners.read_only)
            .any(|candidate| candidate.as_str() == owner);
        if !allowed {
            return Err(GroundFailure::OutOfOwner(query.clone()));
        }
        evidence.push(witness.clone());
    }
    let grounding = Grounding::new(phase.guide.uses.clone()).map_err(GroundFailure::Canonical)?;
    Ok(PhaseGrounding {
        grounding,
        evidence,
        limitations: deck.limitations().to_vec(),
    })
}

fn matches_query(row: &GroundedEvidence, query: &SourceQuery) -> bool {
    match (row, query) {
        (GroundedEvidence::Anchor(a), SourceQuery::Anchor(q)) => &a.id == q,
        (GroundedEvidence::Excerpt(a), SourceQuery::Excerpt(q)) => &a.id == q,
        (GroundedEvidence::Specimen(a), SourceQuery::Specimen(q)) => &a.id == q,
        _ => false,
    }
}
fn evidence_owner(row: &GroundedEvidence) -> &str {
    match row {
        GroundedEvidence::Anchor(v) => &v.owner,
        GroundedEvidence::Excerpt(v) => &v.owner,
        GroundedEvidence::Specimen(v) => &v.owner,
    }
}

#[allow(dead_code)]
fn _owner_type(_: &OwnerId) {}
