use super::*;
use sim_roadmap_core::*;

#[test]
fn ownership_guard_reuses_incremental_core() {
    let roadmap_sources = [
        include_str!("lib.rs"),
        include_str!("compile.rs"),
        include_str!("invalidation.rs"),
        include_str!("../../sim-roadmap-core/src/lib.rs"),
        include_str!("../../sim-roadmap-refine/src/lib.rs"),
    ];
    assert!(roadmap_sources[2].contains("IncrementalEngine"));
    for source in roadmap_sources {
        for forbidden in ["struct Frontier", "struct Memo", "reverse: BTreeMap"] {
            assert!(
                !source.contains(forbidden),
                "second generic graph owner: {forbidden}"
            );
        }
    }
}

#[test]
fn exact_consumes_and_aggregate_requires_are_distinct() {
    let parent = PhaseId::new("parent").unwrap();
    let child = PhaseId::new("child").unwrap();
    let consumer = PhaseId::new("consumer").unwrap();
    let output = OutputId::new("artifact").unwrap();
    let mut spec = minimal_spec(parent.clone());
    spec.phases.get_mut(&parent).unwrap().body = PhaseBody::Composite {
        children: vec![child.clone()],
    };
    spec.phases
        .insert(child.clone(), phase(child.clone(), Some(parent.clone())));
    let mut consuming = phase(consumer.clone(), None);
    consuming.dependencies = vec![
        PhaseDependency::Requires(PhaseRef::Local(parent.clone())),
        PhaseDependency::Consumes(OutputRef {
            phase: PhaseRef::Local(child.clone()),
            output: output.clone(),
        }),
    ];
    spec.phases.get_mut(&child).unwrap().outputs.insert(
        output.clone(),
        OutputContract {
            description: "x".into(),
            content: None,
        },
    );
    spec.phases.insert(consumer.clone(), consuming);
    let mut observations = Observations::default();
    assert_eq!(blockers(&spec, &observations, &consumer).len(), 2);
    observations.completed_phases.insert(child.clone());
    observations.outputs.insert((child, output), "exact".into());
    assert!(blockers(&spec, &observations, &consumer).is_empty());
}

#[test]
fn exact_fact_invalidation_does_not_dirty_unrelated_queries() {
    let a = PlanKey::Source(SourceQueryKey::Anchor("a".into()));
    let b = PlanKey::Source(SourceQueryKey::Anchor("b".into()));
    let qa = PlanKey::Phase(PhaseId::new("a").unwrap());
    let qb = PlanKey::Phase(PhaseId::new("b").unwrap());
    let mut index = DependencyIndex::new();
    index.register_observed(qa.clone(), vec![a.clone()], "a".into());
    index.register_observed(qb.clone(), vec![b], "b".into());
    index.verify(qa.clone()).unwrap();
    index.verify(qb.clone()).unwrap();
    let dirty = index.invalidate(&a);
    assert!(dirty.contains(&qa));
    assert!(!dirty.contains(&qb));
}

fn minimal_spec(root: PhaseId) -> RoadmapSpec {
    let mut phases = std::collections::BTreeMap::new();
    phases.insert(root.clone(), phase(root.clone(), None));
    RoadmapSpec {
        schema: SchemaId::new("s").unwrap(),
        id: RoadmapId::new("r").unwrap(),
        charter: Charter {
            title: "t".into(),
            intent: "i".into(),
        },
        root,
        phases,
        imports: Default::default(),
        limits: Limits::DEFAULT,
    }
}
fn phase(id: PhaseId, parent: Option<PhaseId>) -> PhaseSpec {
    PhaseSpec {
        id,
        parent,
        title: "t".into(),
        intent: "i".into(),
        body: PhaseBody::Leaf {
            checkpoints: vec![],
        },
        dependencies: vec![],
        owners: Default::default(),
        resources: Default::default(),
        effects: Default::default(),
        capabilities: Default::default(),
        changes: Default::default(),
        acceptance: AcceptanceContract {
            policy: ProofPolicy::All,
            statements: Default::default(),
        },
        coverage: vec![],
        outputs: Default::default(),
        guide: Default::default(),
        origin: PhaseOrigin::Authored,
    }
}
