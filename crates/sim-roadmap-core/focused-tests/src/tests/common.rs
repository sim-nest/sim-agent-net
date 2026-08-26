    use std::collections::{BTreeMap, BTreeSet};

    use sim_kernel::{ContentId, Symbol};
    use sim_roadmap_core::*;
    use sim_source_deck::SourceQuery;

    fn content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }

    fn phase(name: &str) -> PhaseSpec {
        PhaseSpec {
            id: PhaseId::new(name).unwrap(),
            parent: None,
            title: "A phase".into(),
            intent: "State intent".into(),
            body: PhaseBody::Leaf {
                checkpoints: vec![],
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
        }
    }

    fn statement(name: &str) -> AcceptanceStatement {
        AcceptanceStatement {
            obligation: ObligationId::new(name).unwrap(),
            subject: sim_kernel::Ref::Symbol(Symbol::qualified("roadmap", name)),
            predicate: Symbol::new("satisfies"),
            object: sim_kernel::Ref::Symbol(Symbol::qualified("roadmap", "acceptance")),
            supporting_refs: vec![],
        }
    }

    fn tree_spec(mut phases: Vec<PhaseSpec>) -> RoadmapSpec {
        let root = PhaseId::new("root").unwrap();
        let phases = phases.drain(..).map(|p| (p.id.clone(), p)).collect();
        RoadmapSpec {
            schema: SchemaId::new("roadmap-v1").unwrap(),
            id: RoadmapId::new("focused-tree").unwrap(),
            charter: Charter { title: "Focused tree".into(), intent: "Prove bounded refinement laws".into() },
            root,
            phases,
            imports: BTreeMap::new(),
            limits: Limits::DEFAULT,
        }
    }

    fn valid_refinement() -> RoadmapSpec {
        let mut root = phase("root");
        let mut leaf = phase("leaf");
        leaf.parent = Some(root.id.clone());
        root.body = PhaseBody::Composite { children: vec![leaf.id.clone()] };
        root.owners.mutable.insert(OwnerId::new("sim-agent-net").unwrap());
        root.resources.resources.insert(ResourceId::new("cpu").unwrap());
        root.capabilities.capabilities.insert(CapabilityId::new("compile").unwrap());
        root.effects.effects.insert(EffectId::new("local-write").unwrap());
        root.changes.targets.insert(ChangeId::new("roadmap-core").unwrap());
        let parent = statement("parent-law");
        root.acceptance.statements.insert(parent.obligation.clone(), parent);
        let child = statement("child-proof");
        leaf.acceptance.statements.insert(child.obligation.clone(), child);
        root.coverage.push(ObligationCoverage::Contributes {
            parent: ObligationId::new("parent-law").unwrap(),
            phase: leaf.id.clone(),
            child: ObligationId::new("child-proof").unwrap(),
        });
        tree_spec(vec![root, leaf])
    }

    fn revision(phases: Vec<PhaseSpec>) -> RoadmapRevision {
        let phases = phases.into_iter().map(|p| (p.id.clone(), p)).collect();
        RoadmapRevision::new(
            None,
            RoadmapSpec {
                schema: SchemaId::new("roadmap-v1").unwrap(),
                id: RoadmapId::new("focused").unwrap(),
                charter: Charter {
                    title: "Focused".into(),
                    intent: "Prove canonical identity".into(),
                },
                root: PhaseId::new("root").unwrap(),
                phases,
                imports: BTreeMap::new(),
                limits: Limits::DEFAULT,
            },
            RevisionChange {
                id: ChangeId::new("initial").unwrap(),
                rationale: "Initial revision".into(),
            },
        )
        .unwrap()
    }
