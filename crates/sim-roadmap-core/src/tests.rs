use super::*;

fn id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
}

fn guide() -> ImplementationGuide {
    let query = SourceQuery::Anchor("anchor/rustdoc/sim-roadmap-core/revision".into());
    let promise = PromiseId::new("revision-api").unwrap();
    ImplementationGuide {
        uses: vec![query.clone()],
        change_targets: vec![ChangeTarget {
            change: ChangeId::new("core-values").unwrap(),
            owner: OwnerId::new("sim-agent-net").unwrap(),
            package: Some("sim-roadmap-core".into()),
            description: "Add pure roadmap values".into(),
        }],
        promises: vec![Promise::PublicDeclaration {
            id: promise.clone(),
            owner: OwnerId::new("sim-agent-net").unwrap(),
            anchor: "anchor/rustdoc/sim-roadmap-core/revision".into(),
        }],
        sketches: vec![AnchoredSketch {
            id: SketchId::new("revision-construction").unwrap(),
            language: SketchLanguage::Rust,
            role: SketchRole::Example,
            body: "let revision = RoadmapRevision::new(parent, spec, change)?;".into(),
            bindings: vec![
                SketchBinding::Uses {
                    label: "revision".into(),
                    query,
                },
                SketchBinding::Produces {
                    label: "api".into(),
                    promise,
                },
            ],
        }],
    }
}

fn phase(id_text: &str, guide: ImplementationGuide) -> PhaseSpec {
    PhaseSpec {
        id: PhaseId::new(id_text).unwrap(),
        parent: None,
        title: "Define values".into(),
        intent: "State bounded intent".into(),
        body: PhaseBody::Leaf {
            checkpoints: vec![CheckpointSpec {
                id: CheckpointId::new("checked").unwrap(),
                statement: "Focused tests pass".into(),
            }],
        },
        dependencies: vec![],
        owners: OwnerEnvelope::default(),
        resources: ResourceEnvelope::default(),
        effects: EffectEnvelope::default(),
        acceptance: AcceptanceContract {
            policy: ProofPolicy::All,
            statements: BTreeMap::new(),
        },
        outputs: BTreeMap::new(),
        guide,
        origin: PhaseOrigin::Authored,
    }
}

fn spec(phases: impl IntoIterator<Item = PhaseSpec>) -> RoadmapSpec {
    let phases: BTreeMap<_, _> = phases.into_iter().map(|p| (p.id.clone(), p)).collect();
    RoadmapSpec {
        schema: SchemaId::new("roadmap-v1").unwrap(),
        id: RoadmapId::new("value-roadmap").unwrap(),
        charter: Charter {
            title: "Roadmap values".into(),
            intent: "Give implementers exact reviewed guidance".into(),
        },
        root: PhaseId::new("root").unwrap(),
        phases,
        imports: BTreeMap::new(),
        limits: Limits::DEFAULT,
    }
}

#[test]
fn insertion_order_does_not_change_revision_identity() {
    let root = phase("root", ImplementationGuide::default());
    let mut leaf = phase("leaf", guide());
    leaf.parent = Some(root.id.clone());
    let change = RevisionChange {
        id: ChangeId::new("initial").unwrap(),
        rationale: "Initial authored revision".into(),
    };
    let a = RoadmapRevision::new(None, spec([root.clone(), leaf.clone()]), change.clone()).unwrap();
    let b = RoadmapRevision::new(None, spec([leaf, root]), change).unwrap();
    assert_eq!(a.id(), b.id());
}

#[test]
fn invalid_binding_and_unbound_promise_fail_before_identity() {
    let mut bad = guide();
    bad.sketches[0].bindings[0] = SketchBinding::Uses {
        label: "missing".into(),
        query: SourceQuery::Anchor("absent".into()),
    };
    assert!(matches!(
        RoadmapRevision::new(
            None,
            spec([phase("root", bad)]),
            RevisionChange {
                id: ChangeId::new("bad").unwrap(),
                rationale: "Rejected change".into()
            }
        ),
        Err(Failure::InvalidBinding { .. })
    ));
    let mut unbound = guide();
    unbound.sketches[0]
        .bindings
        .retain(|b| matches!(b, SketchBinding::Uses { .. }));
    assert!(matches!(
        RoadmapRevision::new(
            None,
            spec([phase("root", unbound)]),
            RevisionChange {
                id: ChangeId::new("bad2").unwrap(),
                rationale: "Rejected change".into()
            }
        ),
        Err(Failure::UnboundPromise(_))
    ));
}

#[test]
fn duplicate_id_unpinned_import_and_over_limit_guide_fail() {
    let mut duplicate = guide();
    duplicate.promises.push(duplicate.promises[0].clone());
    assert!(matches!(
        RoadmapRevision::new(
            None,
            spec([phase("root", duplicate)]),
            RevisionChange {
                id: ChangeId::new("duplicate").unwrap(),
                rationale: "Rejected duplicate".into()
            }
        ),
        Err(Failure::Duplicate {
            kind: "promise",
            ..
        })
    ));
    let mut imported = spec([phase("root", ImplementationGuide::default())]);
    imported.imports.insert(
        ImportId::new("base").unwrap(),
        PinnedRoadmapRef {
            roadmap: RoadmapId::new("base").unwrap(),
            revision: RoadmapRevisionId(id(0)),
            root_phase: PhaseId::new("root").unwrap(),
            root_content: id(0),
        },
    );
    assert!(matches!(
        RoadmapRevision::new(
            None,
            imported,
            RevisionChange {
                id: ChangeId::new("import").unwrap(),
                rationale: "Rejected import".into()
            }
        ),
        Err(Failure::UnpinnedImport(_))
    ));
    let mut over = spec([phase("root", guide())]);
    over.limits.guide_promises = 0;
    assert!(matches!(
        RoadmapRevision::new(
            None,
            over,
            RevisionChange {
                id: ChangeId::new("limit").unwrap(),
                rationale: "Rejected limit".into()
            }
        ),
        Err(Failure::OverLimit {
            limit: "guide_promises",
            ..
        })
    ));
}

#[test]
fn acceptance_projects_to_evidence_bearing_claim() {
    let evidence = Ref::Content(id(7));
    let statement = AcceptanceStatement {
        obligation: ObligationId::new("api-exists").unwrap(),
        subject: Ref::Symbol(Symbol::qualified("roadmap", "revision")),
        predicate: Symbol::new("satisfies"),
        object: Ref::Symbol(Symbol::qualified("roadmap", "acceptance")),
        supporting_refs: vec![evidence.clone()],
    };
    let claim = statement.as_claim();
    assert_eq!(claim.evidence, vec![evidence]);
    assert_eq!(claim.kind, ClaimKind::Asserted);
}

#[test]
fn ids_reject_empty_control_path_like_and_long_text() {
    assert!(RoadmapId::new("").is_err());
    assert!(RoadmapId::new("a\nb").is_err());
    assert!(RoadmapId::new("a/b").is_err());
    assert!(RoadmapId::new("x".repeat(97)).is_err());
}

#[test]
fn claimed_revision_is_verified_without_self_hashing() {
    let change = RevisionChange {
        id: ChangeId::new("initial").unwrap(),
        rationale: "Initial revision".into(),
    };
    let revision = RoadmapRevision::new(
        None,
        spec([phase("root", ImplementationGuide::default())]),
        change.clone(),
    )
    .unwrap();
    let decoded =
        RoadmapRevision::verify_claimed(revision.id().clone(), None, revision.spec.clone(), change)
            .unwrap();
    assert_eq!(decoded.id(), revision.id());
    assert!(matches!(
        RoadmapRevision::verify_claimed(
            RoadmapRevisionId(id(9)),
            None,
            revision.spec,
            RevisionChange {
                id: ChangeId::new("initial").unwrap(),
                rationale: "Initial revision".into()
            }
        ),
        Err(Failure::ClaimedRevisionMismatch)
    ));
}
