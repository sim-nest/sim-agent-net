use std::collections::BTreeMap;

use sim_kernel::{ContentId, Datum, Symbol};
use sim_roadmap_core::{ObligationId, PhaseId, PhaseSpec, RoadmapRevisionId};
use sim_source_deck::SourceQuery;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundingId(pub ContentId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grounding {
    pub id: GroundingId,
    pub resolved: Vec<SourceQuery>,
}

impl Grounding {
    pub fn new(resolved: Vec<SourceQuery>) -> Result<Self, String> {
        let datum = Datum::Node {
            tag: Symbol::qualified("roadmap-refine", "grounding-v1"),
            fields: vec![(
                Symbol::new("resolved"),
                Datum::Vector(resolved.iter().map(source_query_datum).collect()),
            )],
        };
        Ok(Self {
            id: GroundingId(datum.content_id().map_err(|error| error.to_string())?),
            resolved,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildContribution {
    pub child: PhaseId,
    pub obligation: ObligationId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefinementProposal {
    pub base_revision: RoadmapRevisionId,
    pub parent: PhaseId,
    pub expected_parent: ContentId,
    pub expected_grounding: GroundingId,
    pub children: Vec<PhaseSpec>,
    pub coverage: BTreeMap<ObligationId, Vec<ChildContribution>>,
    pub rationale: String,
}

pub fn phase_fingerprint(phase: &PhaseSpec) -> Result<ContentId, String> {
    Datum::Node {
        tag: Symbol::qualified("roadmap-refine", "PhaseFingerprintV2"),
        fields: vec![(Symbol::new("phase"), phase.canonical_datum())],
    }
    .content_id()
    .map_err(|error| error.to_string())
}

fn source_query_datum(query: &SourceQuery) -> Datum {
    let (kind, id) = match query {
        SourceQuery::Anchor(id) => ("anchor", id),
        SourceQuery::Excerpt(id) => ("excerpt", id),
        SourceQuery::Specimen(id) => ("specimen", id),
    };
    Datum::Node {
        tag: Symbol::qualified("roadmap-refine", "SourceQueryV1"),
        fields: vec![
            (Symbol::new("kind"), Datum::Symbol(Symbol::new(kind))),
            (Symbol::new("id"), Datum::String(id.clone())),
        ],
    }
}
