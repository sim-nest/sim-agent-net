#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub id_bytes: usize,
    pub document_bytes: usize,
    pub prose_bytes: usize,
    pub phases: usize,
    pub imports: usize,
    pub outputs_per_phase: usize,
    pub checkpoints_per_phase: usize,
    pub children_per_phase: usize,
    pub tree_depth: usize,
    pub causal_path: usize,
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
        children_per_phase: 256,
        tree_depth: 64,
        causal_path: 64,
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
    Tree {
        rule: &'static str,
        path: CausalPath,
        phase: PhaseId,
        related: Option<PhaseId>,
    },
    Widening {
        ancestor: PhaseId,
        phase: PhaseId,
        field: &'static str,
        value: String,
        path: CausalPath,
    },
    Coverage {
        rule: &'static str,
        phase: PhaseId,
        obligation: ObligationId,
        path: CausalPath,
    },
    CircularCompletion {
        phase: PhaseId,
        dependency: PhaseId,
        path: CausalPath,
    },
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilityEnvelope {
    pub capabilities: BTreeSet<CapabilityId>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeEnvelope {
    pub targets: BTreeSet<ChangeId>,
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
pub enum ObligationCoverage {
    Contributes {
        parent: ObligationId,
        phase: PhaseId,
        child: ObligationId,
    },
    RetainedAtParent {
        parent: ObligationId,
    },
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
    pub capabilities: CapabilityEnvelope,
    pub changes: ChangeEnvelope,
    pub acceptance: AcceptanceContract,
    pub coverage: Vec<ObligationCoverage>,
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
