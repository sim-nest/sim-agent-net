//! Loaded roadmap-refiner conduct and its machine-owned admission boundary.

use std::collections::BTreeMap;

use sim_kernel::{ContentId, Datum, Symbol};
use sim_roadmap_core::{ObligationId, PhaseId, PhaseSpec, RoadmapRevision};
use sim_roadmap_refine::{
    AppliedRefinement, ChildContribution, CompilationHooks, Grounding, RefinementProposal, Refusal,
    TractabilityPolicy, apply_refinement, phase_fingerprint,
};
use sim_source_deck::SourceQuery;

/// The default data-only conduct. Topology, Shapes, Cards, behavior, and its
/// extension seam are separate immutable assets so a replacement changes no runner code.
pub const DEFAULT_REFINER_TOPOLOGY: &str = include_str!("../catalog/roadmap-refiner-v1.simtopo");
/// Strict receive face accepted from a model-backed proposer.
pub const REFINEMENT_PROPOSAL_SHAPE: &str = "roadmap/RefinementProposal-v1";
/// Machine-produced terminal result face.
pub const REFINEMENT_RESULT_SHAPE: &str = "roadmap/RefinementResult-v1";
/// Public Card catalog used by the topology call nodes.
pub const REFINER_CARDS: &str =
    "roadmap/refiner-cards-v1:receive,inspect,propose,validate-refinement,review,revise,finish";
/// Default behavior packet; it contains guidance, never validation authority.
pub const DEFAULT_REFINER_BEHAVIOR: &str = "roadmap/refiner-behavior-v1:propose bounded children; preserve unknowns; never claim rank, proof, completion, mutation, or authority";
/// Stable extension target shared by default and third-party packages.
pub const REFINER_EXTENSION_TARGET: &str = "roadmap/refiner-v1";

/// Content identities for every executable and interpretive package asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefinerPins {
    pub graph: ContentId,
    pub proposal_shape: ContentId,
    pub result_shape: ContentId,
    pub cards: ContentId,
    pub behavior: ContentId,
    pub extension_target: ContentId,
}

/// A loadable refiner package. Third parties supply data under the same public Shapes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefinerPackage<'a> {
    pub topology: &'a str,
    pub proposal_shape: &'a str,
    pub result_shape: &'a str,
    pub cards: &'a str,
    pub behavior: &'a str,
    pub extension_target: &'a str,
    pub pins: RefinerPins,
}

impl RefinerPackage<'static> {
    pub fn default_package() -> Result<Self, String> {
        Self::load(
            DEFAULT_REFINER_TOPOLOGY,
            REFINEMENT_PROPOSAL_SHAPE,
            REFINEMENT_RESULT_SHAPE,
            REFINER_CARDS,
            DEFAULT_REFINER_BEHAVIOR,
            REFINER_EXTENSION_TARGET,
        )
    }
}

impl<'a> RefinerPackage<'a> {
    pub fn load(
        topology: &'a str,
        proposal_shape: &'a str,
        result_shape: &'a str,
        cards: &'a str,
        behavior: &'a str,
        extension_target: &'a str,
    ) -> Result<Self, String> {
        if proposal_shape != REFINEMENT_PROPOSAL_SHAPE
            || result_shape != REFINEMENT_RESULT_SHAPE
            || extension_target != REFINER_EXTENSION_TARGET
        {
            return Err("refiner package changed its public Shape or extension target".into());
        }
        for node in [
            "receive", "inspect", "propose", "validate", "review", "revise", "finish",
        ] {
            if !topology.contains(&format!("node {node} ")) {
                return Err(format!("refiner topology is missing {node}"));
            }
        }
        Ok(Self {
            topology,
            proposal_shape,
            result_shape,
            cards,
            behavior,
            extension_target,
            pins: RefinerPins {
                graph: content_id("graph", topology)?,
                proposal_shape: content_id("shape", proposal_shape)?,
                result_shape: content_id("shape", result_shape)?,
                cards: content_id("cards", cards)?,
                behavior: content_id("behavior", behavior)?,
                extension_target: content_id("extension", extension_target)?,
            },
        })
    }
}

fn content_id(kind: &str, text: &str) -> Result<ContentId, String> {
    Datum::Node {
        tag: Symbol::qualified("roadmap-refiner", "asset-v1"),
        fields: vec![
            (Symbol::new("kind"), Datum::String(kind.into())),
            (Symbol::new("bytes"), Datum::String(text.into())),
        ],
    }
    .content_id()
    .map_err(|error| error.to_string())
}

/// Exact grounded material allowed onto the proposer-facing BRIDGE face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefinerFace {
    pub grounded_parent: String,
    pub implementation_guide: String,
    pub source_deck: String,
    pub derived_profile: String,
    pub atomicity_policy: String,
    pub remaining_bounds: String,
    pub rejection_feedback: Option<String>,
}

impl RefinerFace {
    /// Renders a strict, injection-fenced BRIDGE payload with no ambient authority.
    pub fn render_bridge(&self) -> String {
        fn field(name: &str, value: &str) -> String {
            format!(
                "{name}-bytes={}\n{name}=<untrusted-source>{}</untrusted-source>\n",
                value.len(),
                value
            )
        }
        let mut out = String::from("BRIDGE roadmap/refiner-face-v1\n");
        out.push_str(&field("grounded-parent", &self.grounded_parent));
        out.push_str(&field("implementation-guide", &self.implementation_guide));
        out.push_str(&field("source-deck", &self.source_deck));
        out.push_str(&field("derived-profile", &self.derived_profile));
        out.push_str(&field("atomicity-policy", &self.atomicity_policy));
        out.push_str(&field("remaining-bounds", &self.remaining_bounds));
        out.push_str(&field(
            "rejection-feedback",
            self.rejection_feedback.as_deref().unwrap_or("none"),
        ));
        out
    }
}

/// Only model-authored fields which may cross the receive boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalDraft {
    pub children: Vec<PhaseSpec>,
    pub coverage: BTreeMap<ObligationId, Vec<ChildContribution>>,
    pub rationale: String,
    pub unanswered: Vec<SourceQuery>,
}

/// Result of refiner execution. Unknown source truth cannot be silently erased.
#[derive(Debug)]
pub enum RefinerResult {
    Admitted(Box<AppliedRefinement>),
    Rejected(Refusal),
    Blocked { unanswered: Vec<SourceQuery> },
}

/// Rejects a decoded packet unless its top-level vocabulary is exactly the proposal vocabulary.
/// Decoding children and coverage into typed values remains the installed codec's responsibility.
pub fn check_proposal_fields<'a>(fields: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    const ALLOWED: &[&str] = &["children", "coverage", "rationale", "unanswered"];
    const FORBIDDEN: &[&str] = &[
        "profile",
        "profile-counts",
        "rank",
        "certificate",
        "completion",
        "complete",
        "mutation",
        "mutation-bytes",
        "proof",
        "proof-verdict",
        "authority",
        "authority-request",
    ];
    let mut seen = std::collections::BTreeSet::new();
    for field in fields {
        if FORBIDDEN.contains(&field) {
            return Err(format!("model-authored {field} is forbidden"));
        }
        if !ALLOWED.contains(&field) {
            return Err(format!("unknown RefinementProposal field {field}"));
        }
        if !seen.insert(field) {
            return Err(format!("duplicate RefinementProposal field {field}"));
        }
    }
    for required in ["children", "coverage", "rationale", "unanswered"] {
        if !seen.contains(required) {
            return Err(format!("missing RefinementProposal field {required}"));
        }
    }
    Ok(())
}

/// Regrounds a proposal and delegates all admission facts to the public validator.
pub fn validate_refinement(
    base: &RoadmapRevision,
    parent: PhaseId,
    grounding: &Grounding,
    expected_grounding: &sim_roadmap_refine::GroundingId,
    policy: &TractabilityPolicy,
    draft: ProposalDraft,
    hooks: &dyn CompilationHooks,
) -> RefinerResult {
    if !draft.unanswered.is_empty() {
        return RefinerResult::Blocked {
            unanswered: draft.unanswered,
        };
    }
    if &grounding.id != expected_grounding {
        return RefinerResult::Rejected(Refusal::StaleGrounding);
    }
    let Some(parent_spec) = base.spec.phases.get(&parent) else {
        return RefinerResult::Rejected(Refusal::MissingParent(parent));
    };
    let expected_parent = match phase_fingerprint(parent_spec) {
        Ok(identity) => identity,
        Err(error) => return RefinerResult::Rejected(Refusal::OutputCompilation(error)),
    };
    let proposal = RefinementProposal {
        base_revision: base.id().clone(),
        parent,
        expected_parent,
        expected_grounding: grounding.id.clone(),
        children: draft.children,
        coverage: draft.coverage,
        rationale: draft.rationale,
    };
    match apply_refinement(base, grounding, policy, proposal, hooks) {
        Ok(applied) if applied.certificate.verify() => RefinerResult::Admitted(Box::new(applied)),
        Ok(_) => RefinerResult::Rejected(Refusal::OutputCompilation(
            "invalid descent certificate".into(),
        )),
        Err(refusal) => RefinerResult::Rejected(refusal),
    }
}

#[cfg(test)]
#[path = "refiner_tests.rs"]
mod tests;
// conformance: bounded data-only roadmap refiner conduct.
