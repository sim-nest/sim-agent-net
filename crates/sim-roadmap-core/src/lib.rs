#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{Claim, ClaimKind, ContentId, Datum, Ref, Symbol, Visibility};
use sim_source_deck::SourceQuery;

mod id;
pub use id::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub id_bytes: usize,
    pub document_bytes: usize,
    pub prose_bytes: usize,
    pub phases: usize,
    pub imports: usize,
    pub outputs_per_phase: usize,
    pub checkpoints_per_phase: usize,
    pub guide_queries: usize,
    pub guide_targets: usize,
    pub guide_promises: usize,
    pub guide_sketches: usize,
    pub sketch_bytes: usize,
    pub sketch_bindings: usize,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        id_bytes: 96,
        document_bytes: 1_000_000,
        prose_bytes: 16_384,
        phases: 1_024,
        imports: 128,
        outputs_per_phase: 128,
        checkpoints_per_phase: 256,
        guide_queries: 128,
        guide_targets: 128,
        guide_promises: 128,
        guide_sketches: 64,
        sketch_bytes: 8_192,
        sketch_bindings: 128,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    InvalidText {
        kind: &'static str,
        reason: &'static str,
    },
    OverLimit {
        limit: &'static str,
        actual: usize,
        maximum: usize,
    },
    Duplicate {
        kind: &'static str,
        id: String,
    },
    Missing {
        kind: &'static str,
        id: String,
    },
    InvalidBinding {
        sketch: SketchId,
        label: String,
    },
    UnboundPromise(PromiseId),
    UnpinnedImport(ImportId),
    ClaimedRevisionMismatch,
    Canonical(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Charter {
    pub title: String,
    pub intent: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointSpec {
    pub id: CheckpointId,
    pub statement: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseBody {
    Leaf { checkpoints: Vec<CheckpointSpec> },
    Composite { children: Vec<PhaseId> },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PhaseRef {
    Local(PhaseId),
    Imported {
        import: ImportId,
        phase: PhaseId,
        phase_content: ContentId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OutputRef {
    pub phase: PhaseRef,
    pub output: OutputId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseDependency {
    Requires(PhaseRef),
    Consumes(OutputRef),
    PrefersAfter(PhaseRef),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnerEnvelope {
    pub mutable: BTreeSet<OwnerId>,
    pub read_only: BTreeSet<OwnerId>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceEnvelope {
    pub resources: BTreeSet<ResourceId>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectEnvelope {
    pub effects: BTreeSet<EffectId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofPolicy {
    All,
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceStatement {
    pub obligation: ObligationId,
    pub subject: Ref,
    pub predicate: Symbol,
    pub object: Ref,
    pub supporting_refs: Vec<Ref>,
}

impl AcceptanceStatement {
    pub fn as_claim(&self) -> Claim {
        Claim::new(
            self.subject.clone(),
            self.predicate.clone(),
            self.object.clone(),
        )
        .with_kind(ClaimKind::Asserted)
        .with_evidence(self.supporting_refs.clone())
        .with_visibility(Visibility::Public)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceContract {
    pub policy: ProofPolicy,
    pub statements: BTreeMap<ObligationId, AcceptanceStatement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputContract {
    pub description: String,
    pub content: Option<ContentId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedRoadmapRef {
    pub roadmap: RoadmapId,
    pub revision: RoadmapRevisionId,
    pub root_phase: PhaseId,
    pub root_content: ContentId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseOrigin {
    Authored,
    Imported {
        import: ImportId,
        phase: PhaseId,
        phase_content: ContentId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeTarget {
    pub change: ChangeId,
    pub owner: OwnerId,
    pub package: Option<String>,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Promise {
    PublicDeclaration {
        id: PromiseId,
        owner: OwnerId,
        anchor: String,
    },
    SourcePostimage {
        id: PromiseId,
        owner: OwnerId,
        path: String,
        content: ContentId,
    },
    CheckedSpecimen {
        id: PromiseId,
        owner: OwnerId,
        specimen: String,
    },
    ProducedOutput {
        id: PromiseId,
        output: OutputId,
    },
    Acceptance {
        id: PromiseId,
        obligation: ObligationId,
    },
}
impl Promise {
    pub fn id(&self) -> &PromiseId {
        match self {
            Self::PublicDeclaration { id, .. }
            | Self::SourcePostimage { id, .. }
            | Self::CheckedSpecimen { id, .. }
            | Self::ProducedOutput { id, .. }
            | Self::Acceptance { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchLanguage {
    Rust,
    Sim,
    Text,
    Other(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchRole {
    Pattern,
    Interface,
    Example,
    Constraint,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchBinding {
    Uses { label: String, query: SourceQuery },
    Produces { label: String, promise: PromiseId },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchoredSketch {
    pub id: SketchId,
    pub language: SketchLanguage,
    pub role: SketchRole,
    pub body: String,
    pub bindings: Vec<SketchBinding>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImplementationGuide {
    pub uses: Vec<SourceQuery>,
    pub change_targets: Vec<ChangeTarget>,
    pub promises: Vec<Promise>,
    pub sketches: Vec<AnchoredSketch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhaseSpec {
    pub id: PhaseId,
    pub parent: Option<PhaseId>,
    pub title: String,
    pub intent: String,
    pub body: PhaseBody,
    pub dependencies: Vec<PhaseDependency>,
    pub owners: OwnerEnvelope,
    pub resources: ResourceEnvelope,
    pub effects: EffectEnvelope,
    pub acceptance: AcceptanceContract,
    pub outputs: BTreeMap<OutputId, OutputContract>,
    pub guide: ImplementationGuide,
    pub origin: PhaseOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadmapSpec {
    pub schema: SchemaId,
    pub id: RoadmapId,
    pub charter: Charter,
    pub root: PhaseId,
    pub phases: BTreeMap<PhaseId, PhaseSpec>,
    pub imports: BTreeMap<ImportId, PinnedRoadmapRef>,
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionChange {
    pub id: ChangeId,
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadmapRevision {
    id: RoadmapRevisionId,
    pub parent: Option<RoadmapRevisionId>,
    pub spec: RoadmapSpec,
    pub change: RevisionChange,
}

impl RoadmapRevision {
    pub fn new(
        parent: Option<RoadmapRevisionId>,
        spec: RoadmapSpec,
        change: RevisionChange,
    ) -> Result<Self, Failure> {
        validate_spec(&spec)?;
        validate_prose(
            "change rationale",
            &change.rationale,
            spec.limits.prose_bytes,
        )?;
        let datum = revision_datum(parent.as_ref(), &spec, &change);
        let id = RoadmapRevisionId(
            datum
                .content_id()
                .map_err(|e| Failure::Canonical(e.to_string()))?,
        );
        Ok(Self {
            id,
            parent,
            spec,
            change,
        })
    }
    pub fn verify_claimed(
        claimed: RoadmapRevisionId,
        parent: Option<RoadmapRevisionId>,
        spec: RoadmapSpec,
        change: RevisionChange,
    ) -> Result<Self, Failure> {
        let revision = Self::new(parent, spec, change)?;
        if revision.id != claimed {
            return Err(Failure::ClaimedRevisionMismatch);
        }
        Ok(revision)
    }
    pub fn id(&self) -> &RoadmapRevisionId {
        &self.id
    }
    pub fn canonical_datum(&self) -> Datum {
        revision_datum(self.parent.as_ref(), &self.spec, &self.change)
    }
}

fn validate_spec(spec: &RoadmapSpec) -> Result<(), Failure> {
    let l = spec.limits;
    bounded("phases", spec.phases.len(), l.phases)?;
    bounded("imports", spec.imports.len(), l.imports)?;
    validate_prose("charter title", &spec.charter.title, l.prose_bytes)?;
    validate_prose("charter intent", &spec.charter.intent, l.document_bytes)?;
    if !spec.phases.contains_key(&spec.root) {
        return Err(Failure::Missing {
            kind: "root phase",
            id: spec.root.to_string(),
        });
    }
    for (key, phase) in &spec.phases {
        if key != &phase.id {
            return Err(Failure::Duplicate {
                kind: "phase id",
                id: phase.id.to_string(),
            });
        }
        validate_phase(phase, spec)?;
    }
    for (id, pin) in &spec.imports {
        if pin.revision.0.bytes == [0; 32] || pin.root_content.bytes == [0; 32] {
            return Err(Failure::UnpinnedImport(id.clone()));
        }
    }
    Ok(())
}

fn validate_phase(phase: &PhaseSpec, spec: &RoadmapSpec) -> Result<(), Failure> {
    let l = spec.limits;
    validate_prose("phase title", &phase.title, l.prose_bytes)?;
    validate_prose("phase intent", &phase.intent, l.prose_bytes)?;
    bounded(
        "outputs_per_phase",
        phase.outputs.len(),
        l.outputs_per_phase,
    )?;
    if let PhaseBody::Leaf { checkpoints } = &phase.body {
        bounded(
            "checkpoints_per_phase",
            checkpoints.len(),
            l.checkpoints_per_phase,
        )?;
        unique(checkpoints.iter().map(|x| x.id.to_string()), "checkpoint")?;
        for c in checkpoints {
            validate_prose("checkpoint", &c.statement, l.prose_bytes)?;
        }
    }
    validate_guide(&phase.guide, l)?;
    for dep in &phase.dependencies {
        validate_dependency(dep, spec)?;
    }
    if let PhaseOrigin::Imported { import, .. } = &phase.origin {
        if !spec.imports.contains_key(import) {
            return Err(Failure::UnpinnedImport(import.clone()));
        }
    }
    Ok(())
}

fn validate_dependency(dep: &PhaseDependency, spec: &RoadmapSpec) -> Result<(), Failure> {
    let r = match dep {
        PhaseDependency::Requires(r) | PhaseDependency::PrefersAfter(r) => r,
        PhaseDependency::Consumes(o) => &o.phase,
    };
    match r {
        PhaseRef::Local(id) if !spec.phases.contains_key(id) => Err(Failure::Missing {
            kind: "local phase",
            id: id.to_string(),
        }),
        PhaseRef::Imported { import, .. } if !spec.imports.contains_key(import) => {
            Err(Failure::UnpinnedImport(import.clone()))
        }
        _ => Ok(()),
    }
}

fn validate_guide(g: &ImplementationGuide, l: Limits) -> Result<(), Failure> {
    bounded("guide_queries", g.uses.len(), l.guide_queries)?;
    bounded("guide_targets", g.change_targets.len(), l.guide_targets)?;
    bounded("guide_promises", g.promises.len(), l.guide_promises)?;
    bounded("guide_sketches", g.sketches.len(), l.guide_sketches)?;
    unique(g.promises.iter().map(|p| p.id().to_string()), "promise")?;
    unique(g.sketches.iter().map(|s| s.id.to_string()), "sketch")?;
    let promise_ids: BTreeSet<_> = g.promises.iter().map(Promise::id).collect();
    let mut bound = BTreeSet::new();
    for s in &g.sketches {
        validate_prose("sketch", &s.body, l.sketch_bytes)?;
        bounded("sketch_bindings", s.bindings.len(), l.sketch_bindings)?;
        unique(
            s.bindings.iter().map(|b| match b {
                SketchBinding::Uses { label, .. } | SketchBinding::Produces { label, .. } => {
                    label.clone()
                }
            }),
            "binding label",
        )?;
        for b in &s.bindings {
            match b {
                SketchBinding::Uses { label, query } if !g.uses.contains(query) => {
                    return Err(Failure::InvalidBinding {
                        sketch: s.id.clone(),
                        label: label.clone(),
                    });
                }
                SketchBinding::Produces { label, promise } if !promise_ids.contains(promise) => {
                    return Err(Failure::InvalidBinding {
                        sketch: s.id.clone(),
                        label: label.clone(),
                    });
                }
                SketchBinding::Produces { promise, .. } => {
                    bound.insert(promise.clone());
                }
                _ => {}
            }
        }
    }
    for p in promise_ids {
        if !bound.contains(p) {
            return Err(Failure::UnboundPromise(p.clone()));
        }
    }
    Ok(())
}

fn validate_prose(kind: &'static str, s: &str, max: usize) -> Result<(), Failure> {
    if s.trim().is_empty() {
        return Err(Failure::InvalidText {
            kind,
            reason: "empty",
        });
    }
    if s.chars().any(|c| c == '\0') {
        return Err(Failure::InvalidText {
            kind,
            reason: "NUL",
        });
    }
    bounded(kind, s.len(), max)
}
fn bounded(limit: &'static str, actual: usize, maximum: usize) -> Result<(), Failure> {
    if actual > maximum {
        Err(Failure::OverLimit {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}
fn unique<I: IntoIterator<Item = String>>(items: I, kind: &'static str) -> Result<(), Failure> {
    let mut seen = BTreeSet::new();
    for id in items {
        if !seen.insert(id.clone()) {
            return Err(Failure::Duplicate { kind, id });
        }
    }
    Ok(())
}

fn revision_datum(
    parent: Option<&RoadmapRevisionId>,
    spec: &RoadmapSpec,
    change: &RevisionChange,
) -> Datum {
    Datum::Node {
        tag: tag("revision-v1"),
        fields: vec![
            (
                Symbol::new("schema"),
                Datum::String(spec.schema.to_string()),
            ),
            (
                Symbol::new("parent"),
                parent.map(|x| content(&x.0)).unwrap_or(Datum::Nil),
            ),
            (Symbol::new("roadmap"), spec_datum(spec)),
            (
                Symbol::new("change"),
                Datum::Node {
                    tag: tag("change-v1"),
                    fields: vec![
                        (Symbol::new("id"), Datum::String(change.id.to_string())),
                        (
                            Symbol::new("rationale"),
                            Datum::String(change.rationale.clone()),
                        ),
                    ],
                },
            ),
        ],
    }
}
fn spec_datum(s: &RoadmapSpec) -> Datum {
    Datum::Node {
        tag: tag("roadmap-spec-v1"),
        fields: vec![
            (Symbol::new("id"), Datum::String(s.id.to_string())),
            (
                Symbol::new("charter"),
                Datum::Vector(vec![
                    Datum::String(s.charter.title.clone()),
                    Datum::String(s.charter.intent.clone()),
                ]),
            ),
            (Symbol::new("root"), Datum::String(s.root.to_string())),
            (
                Symbol::new("imports"),
                Datum::Map(
                    s.imports
                        .iter()
                        .map(|(k, v)| {
                            (
                                Datum::String(k.to_string()),
                                Datum::Vector(vec![
                                    Datum::String(v.roadmap.to_string()),
                                    content(&v.revision.0),
                                    Datum::String(v.root_phase.to_string()),
                                    content(&v.root_content),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
            (
                Symbol::new("phases"),
                Datum::Map(
                    s.phases
                        .iter()
                        .map(|(k, v)| (Datum::String(k.to_string()), phase_datum(v)))
                        .collect(),
                ),
            ),
        ],
    }
}
fn phase_datum(p: &PhaseSpec) -> Datum {
    Datum::Node {
        tag: tag("phase-v1"),
        fields: vec![
            (Symbol::new("id"), Datum::String(p.id.to_string())),
            (
                Symbol::new("parent"),
                p.parent
                    .as_ref()
                    .map(|x| Datum::String(x.to_string()))
                    .unwrap_or(Datum::Nil),
            ),
            (Symbol::new("title"), Datum::String(p.title.clone())),
            (Symbol::new("intent"), Datum::String(p.intent.clone())),
            (
                Symbol::new("semantic"),
                Datum::String(format!(
                    "{:?}",
                    (
                        &p.body,
                        &p.dependencies,
                        &p.owners,
                        &p.resources,
                        &p.effects,
                        &p.acceptance,
                        &p.outputs,
                        &p.guide,
                        &p.origin
                    )
                )),
            ),
        ],
    }
}
fn content(id: &ContentId) -> Datum {
    Datum::String(format!("{:?}", id))
}
fn tag(name: &str) -> Symbol {
    Symbol::qualified("roadmap", name)
}

#[cfg(test)]
mod tests;
