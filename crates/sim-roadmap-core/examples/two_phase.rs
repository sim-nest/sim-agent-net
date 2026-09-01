use sim_roadmap_core::*;
use sim_source_deck::SourceQuery;
use std::collections::BTreeMap;

fn main() -> Result<(), Failure> {
    let root_id = PhaseId::new("root")?;
    let leaf_id = PhaseId::new("values")?;
    let anchor = SourceQuery::Anchor("anchor/rustdoc/sim-roadmap-core/revision".into());
    let excerpt = SourceQuery::Excerpt("excerpt/revision-constructor".into());
    let promise_id = PromiseId::new("revision-api")?;
    let leaf = PhaseSpec {
        id: leaf_id.clone(),
        parent: Some(root_id.clone()),
        title: "Define roadmap values".into(),
        intent: "Give a small implementer exact reviewed guidance".into(),
        body: PhaseBody::Leaf {
            checkpoints: vec![CheckpointSpec {
                id: CheckpointId::new("semantic-tests")?,
                statement: "Canonical and rejection laws pass".into(),
            }],
        },
        dependencies: vec![],
        owners: OwnerEnvelope::default(),
        resources: ResourceEnvelope::default(),
        effects: EffectEnvelope::default(),
        capabilities: CapabilityEnvelope::default(),
        changes: ChangeEnvelope::default(),
        acceptance: AcceptanceContract {
            policy: ProofPolicy::All,
            statements: BTreeMap::new(),
        },
        coverage: vec![],
        outputs: BTreeMap::new(),
        guide: ImplementationGuide {
            uses: vec![anchor.clone(), excerpt.clone()],
            change_targets: vec![ChangeTarget {
                change: ChangeId::new("roadmap-values")?,
                owner: OwnerId::new("sim-agent-net")?,
                package: Some("sim-roadmap-core".into()),
                description: "Add immutable roadmap revision values".into(),
            }],
            promises: vec![Promise::PublicDeclaration {
                id: promise_id.clone(),
                owner: OwnerId::new("sim-agent-net")?,
                anchor: "anchor/rustdoc/sim-roadmap-core/revision".into(),
            }],
            sketches: vec![AnchoredSketch {
                id: SketchId::new("construct-revision")?,
                language: SketchLanguage::Rust,
                role: SketchRole::Example,
                body: "let revision = RoadmapRevision::new(parent, spec, change)?;".into(),
                bindings: vec![
                    SketchBinding::Uses {
                        label: "public-api".into(),
                        query: anchor,
                    },
                    SketchBinding::Uses {
                        label: "constructor-source".into(),
                        query: excerpt,
                    },
                    SketchBinding::Produces {
                        label: "revision-api".into(),
                        promise: promise_id,
                    },
                ],
            }],
        },
        origin: PhaseOrigin::Authored,
    };
    let root = PhaseSpec {
        id: root_id.clone(),
        parent: None,
        title: "Roadmap value framework".into(),
        intent: "State bounded authored intent".into(),
        body: PhaseBody::Composite {
            children: vec![leaf_id.clone()],
        },
        dependencies: vec![],
        owners: OwnerEnvelope::default(),
        resources: ResourceEnvelope::default(),
        effects: EffectEnvelope::default(),
        capabilities: CapabilityEnvelope::default(),
        changes: ChangeEnvelope::default(),
        acceptance: AcceptanceContract {
            policy: ProofPolicy::All,
            statements: BTreeMap::new(),
        },
        coverage: vec![],
        outputs: BTreeMap::new(),
        guide: ImplementationGuide::default(),
        origin: PhaseOrigin::Authored,
    };
    let phases = [(root_id.clone(), root), (leaf_id, leaf)]
        .into_iter()
        .collect();
    let spec = RoadmapSpec {
        schema: SchemaId::new("roadmap-v1")?,
        id: RoadmapId::new("roadmap-values")?,
        charter: Charter {
            title: "Roadmap values".into(),
            intent: "One pure value states intent and reviewed guidance without claiming proof"
                .into(),
        },
        root: root_id,
        phases,
        imports: BTreeMap::new(),
        limits: Limits::DEFAULT,
    };
    let revision = RoadmapRevision::new(
        None,
        spec,
        RevisionChange {
            id: ChangeId::new("initial")?,
            rationale: "Publish the two-phase value example".into(),
        },
    )?;
    println!("{:?}", revision.id());
    Ok(())
}
