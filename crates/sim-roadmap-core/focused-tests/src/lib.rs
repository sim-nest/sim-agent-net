// conformance: bounded roadmap admission laws.

#[cfg(test)]
mod tests {
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

    #[test]
    fn semantic_insertion_order_is_canonical() {
        let mut root = phase("root");
        let mut leaf = phase("leaf");
        leaf.parent = Some(root.id.clone());
        root.body = PhaseBody::Composite {
            children: vec![leaf.id.clone()],
        };
        assert_eq!(
            revision(vec![root.clone(), leaf.clone()]).id(),
            revision(vec![leaf, root]).id()
        );
    }

    #[test]
    fn invalid_ids_and_over_limit_documents_fail_closed() {
        assert!(RoadmapId::new("../roadmap").is_err());
        let mut root = phase("root");
        root.intent = "x".repeat(17_000);
        let phases = [(root.id.clone(), root)].into_iter().collect();
        let result = RoadmapRevision::new(
            None,
            RoadmapSpec {
                schema: SchemaId::new("roadmap-v1").unwrap(),
                id: RoadmapId::new("focused").unwrap(),
                charter: Charter {
                    title: "Focused".into(),
                    intent: "Bounded".into(),
                },
                root: PhaseId::new("root").unwrap(),
                phases,
                imports: BTreeMap::new(),
                limits: Limits::DEFAULT,
            },
            RevisionChange {
                id: ChangeId::new("initial").unwrap(),
                rationale: "Rejected".into(),
            },
        );
        assert!(matches!(result, Err(Failure::OverLimit { .. })));
    }

    #[test]
    fn guide_bindings_duplicates_and_import_pins_fail_before_identity() {
        let promise_id = PromiseId::new("public-api").unwrap();
        let query = SourceQuery::Anchor("anchor/rustdoc/sim-roadmap-core/revision".into());
        let mut root = phase("root");
        root.guide = ImplementationGuide {
            uses: vec![query.clone()],
            change_targets: vec![],
            promises: vec![Promise::PublicDeclaration {
                id: promise_id.clone(),
                owner: OwnerId::new("sim-agent-net").unwrap(),
                anchor: "anchor/rustdoc/sim-roadmap-core/revision".into(),
            }],
            sketches: vec![AnchoredSketch {
                id: SketchId::new("guide").unwrap(),
                language: SketchLanguage::Rust,
                role: SketchRole::Example,
                body: "RoadmapRevision::new(parent, spec, change)".into(),
                bindings: vec![
                    SketchBinding::Uses {
                        label: "wrong".into(),
                        query: SourceQuery::Anchor("missing".into()),
                    },
                    SketchBinding::Produces {
                        label: "api".into(),
                        promise: promise_id.clone(),
                    },
                ],
            }],
        };
        let phases = [(root.id.clone(), root.clone())].into_iter().collect();
        let spec = RoadmapSpec {
            schema: SchemaId::new("roadmap-v1").unwrap(),
            id: RoadmapId::new("focused").unwrap(),
            charter: Charter {
                title: "Focused".into(),
                intent: "Reject invalid bindings".into(),
            },
            root: root.id.clone(),
            phases,
            imports: BTreeMap::new(),
            limits: Limits::DEFAULT,
        };
        assert!(matches!(
            RoadmapRevision::new(
                None,
                spec,
                RevisionChange {
                    id: ChangeId::new("invalid").unwrap(),
                    rationale: "Must fail".into()
                }
            ),
            Err(Failure::InvalidBinding { .. })
        ));

        root.guide.sketches[0].bindings = vec![SketchBinding::Uses {
            label: "anchor".into(),
            query,
        }];
        let phases = [(root.id.clone(), root.clone())].into_iter().collect();
        let spec = RoadmapSpec {
            schema: SchemaId::new("roadmap-v1").unwrap(),
            id: RoadmapId::new("focused").unwrap(),
            charter: Charter {
                title: "Focused".into(),
                intent: "Reject unbound promises".into(),
            },
            root: root.id.clone(),
            phases,
            imports: BTreeMap::new(),
            limits: Limits::DEFAULT,
        };
        assert!(matches!(
            RoadmapRevision::new(
                None,
                spec,
                RevisionChange {
                    id: ChangeId::new("unbound").unwrap(),
                    rationale: "Must fail".into()
                }
            ),
            Err(Failure::UnboundPromise(_))
        ));

        root.guide.promises.push(root.guide.promises[0].clone());
        let phases = [(root.id.clone(), root.clone())].into_iter().collect();
        let spec = RoadmapSpec {
            schema: SchemaId::new("roadmap-v1").unwrap(),
            id: RoadmapId::new("focused").unwrap(),
            charter: Charter {
                title: "Focused".into(),
                intent: "Reject duplicates".into(),
            },
            root: root.id.clone(),
            phases,
            imports: BTreeMap::new(),
            limits: Limits::DEFAULT,
        };
        assert!(matches!(
            RoadmapRevision::new(
                None,
                spec,
                RevisionChange {
                    id: ChangeId::new("duplicate").unwrap(),
                    rationale: "Must fail".into()
                }
            ),
            Err(Failure::Duplicate { .. })
        ));

        let root = phase("root");
        let phases = [(root.id.clone(), root)].into_iter().collect();
        let imports = [(
            ImportId::new("base").unwrap(),
            PinnedRoadmapRef {
                roadmap: RoadmapId::new("base").unwrap(),
                revision: RoadmapRevisionId(content(0)),
                root_phase: PhaseId::new("root").unwrap(),
                root_content: content(0),
            },
        )]
        .into_iter()
        .collect();
        let spec = RoadmapSpec {
            schema: SchemaId::new("roadmap-v1").unwrap(),
            id: RoadmapId::new("focused").unwrap(),
            charter: Charter {
                title: "Focused".into(),
                intent: "Reject unpinned imports".into(),
            },
            root: PhaseId::new("root").unwrap(),
            phases,
            imports,
            limits: Limits::DEFAULT,
        };
        assert!(matches!(
            RoadmapRevision::new(
                None,
                spec,
                RevisionChange {
                    id: ChangeId::new("import").unwrap(),
                    rationale: "Must fail".into()
                }
            ),
            Err(Failure::UnpinnedImport(_))
        ));
    }

    #[test]
    fn local_and_imported_phase_references_are_distinct() {
        let local = PhaseRef::Local(PhaseId::new("leaf").unwrap());
        let imported = PhaseRef::Imported {
            import: ImportId::new("base").unwrap(),
            phase: PhaseId::new("leaf").unwrap(),
            phase_content: content(4),
        };
        assert_ne!(local, imported);
    }

    #[test]
    fn tree_admission_rejects_every_malformed_relationship() {
        let mut missing = phase("root");
        missing.parent = Some(PhaseId::new("absent").unwrap());
        assert!(matches!(tree_spec(vec![missing]).admit(), Err(Failure::Tree { rule: "root-has-parent" | "exactly-one-root", .. })));

        let mut empty = phase("root");
        empty.body = PhaseBody::Composite { children: vec![] };
        assert!(matches!(tree_spec(vec![empty]).admit(), Err(Failure::Tree { rule: "empty-composite", .. })));

        let mut root = phase("root");
        let mut child = phase("child");
        child.parent = Some(root.id.clone());
        root.body = PhaseBody::Composite { children: vec![child.id.clone(), child.id.clone()] };
        assert!(matches!(tree_spec(vec![root, child]).admit(), Err(Failure::Duplicate { kind: "child", .. })));

        let mut root = phase("root");
        let mut disconnected = phase("other");
        disconnected.parent = Some(root.id.clone());
        root.body = PhaseBody::Leaf { checkpoints: vec![] };
        assert!(matches!(tree_spec(vec![root, disconnected]).admit(), Err(Failure::Tree { rule: "disconnected-phase", .. })));
    }

    #[test]
    fn envelopes_narrow_and_authored_patches_remain_separate() {
        let spec = valid_refinement();
        let admitted = spec.admit().unwrap();
        let leaf = &admitted.phases[&PhaseId::new("leaf").unwrap()];
        assert!(leaf.authored_owners.mutable.is_empty());
        assert_eq!(leaf.effective_owners.mutable, spec.phases[&spec.root].owners.mutable);
        assert_eq!(leaf.effective_resources, spec.phases[&spec.root].resources);
        assert_eq!(leaf.effective_capabilities, spec.phases[&spec.root].capabilities);
        assert_eq!(leaf.effective_effects, spec.phases[&spec.root].effects);
        assert_eq!(leaf.effective_changes, spec.phases[&spec.root].changes);

        type WideningMutation = (&'static str, fn(&mut PhaseSpec));
        let widenings: [WideningMutation; 5] = [
            ("owners.mutable", |p| { p.owners.mutable.insert(OwnerId::new("other").unwrap()); }),
            ("resources", |p| { p.resources.resources.insert(ResourceId::new("gpu").unwrap()); }),
            ("capabilities", |p| { p.capabilities.capabilities.insert(CapabilityId::new("network").unwrap()); }),
            ("effects", |p| { p.effects.effects.insert(EffectId::new("publish").unwrap()); }),
            ("change-targets", |p| { p.changes.targets.insert(ChangeId::new("other-crate").unwrap()); }),
        ];
        for (field, mutate) in widenings {
            let mut candidate = valid_refinement();
            mutate(candidate.phases.get_mut(&PhaseId::new("leaf").unwrap()).unwrap());
            assert!(matches!(candidate.admit(), Err(Failure::Widening { field: actual, ref path, .. }) if actual == field && path.phases().iter().map(ToString::to_string).collect::<Vec<_>>() == ["root", "leaf"]));
        }
    }

    #[test]
    fn aggregate_completion_retains_parent_acceptance_and_checks_coverage() {
        let spec = valid_refinement();
        let admitted = spec.admit().unwrap();
        let leaf = &admitted.phases[&PhaseId::new("leaf").unwrap()];
        assert_eq!(leaf.acceptance.inherited, vec![(spec.root.clone(), spec.phases[&spec.root].acceptance.clone())]);

        let mut dropped = valid_refinement();
        dropped.phases.get_mut(&dropped.root.clone()).unwrap().coverage.clear();
        assert!(matches!(dropped.admit(), Err(Failure::Coverage { rule: "dropped-parent-obligation", .. })));

        let mut invented = valid_refinement();
        invented.phases.get_mut(&invented.root.clone()).unwrap().coverage[0] = ObligationCoverage::Contributes {
            parent: ObligationId::new("parent-law").unwrap(), phase: PhaseId::new("leaf").unwrap(), child: ObligationId::new("invented").unwrap()
        };
        assert!(matches!(invented.admit(), Err(Failure::Coverage { rule: "invented-child-obligation", .. })));
    }

    #[test]
    fn descendant_dependencies_and_deep_trees_fail_with_bounded_paths() {
        let mut circular = valid_refinement();
        circular.phases.get_mut(&circular.root.clone()).unwrap().dependencies.push(
            PhaseDependency::Requires(PhaseRef::Local(PhaseId::new("leaf").unwrap()))
        );
        assert!(matches!(circular.admit(), Err(Failure::CircularCompletion { ref path, .. }) if path.phases() == [PhaseId::new("root").unwrap()]));

        let mut phases = vec![];
        for n in 0..6 {
            let name = if n == 0 { "root".to_string() } else { format!("p{n}") };
            let mut p = phase(&name);
            if n > 0 { p.parent = Some(PhaseId::new(if n == 1 { "root".into() } else { format!("p{}", n - 1) }).unwrap()); }
            if n < 5 { p.body = PhaseBody::Composite { children: vec![PhaseId::new(format!("p{}", n + 1)).unwrap()] }; }
            phases.push(p);
        }
        let mut deep = tree_spec(phases);
        deep.limits.tree_depth = 4;
        assert!(matches!(deep.admit(), Err(Failure::Tree { rule: "tree-depth-limit", ref path, .. }) if path.phases().len() == 5));
    }

    #[test]
    fn generated_finite_trees_and_map_permutations_are_deterministic() {
        for width in 1..16 {
            let mut root = phase("root");
            let mut leaves = Vec::new();
            for n in 0..width {
                let mut leaf = phase(&format!("leaf-{n:02}"));
                leaf.parent = Some(root.id.clone());
                leaves.push(leaf);
            }
            root.body = PhaseBody::Composite { children: leaves.iter().map(|p| p.id.clone()).rev().collect() };
            let mut ordered = vec![root.clone()]; ordered.extend(leaves.clone());
            let mut reversed = leaves; reversed.reverse(); reversed.push(root);
            let a = tree_spec(ordered).admit().unwrap();
            let b = tree_spec(reversed).admit().unwrap();
            assert_eq!(a, b);
        }

        let mut failures = BTreeSet::new();
        for reverse in [false, true] {
            let mut candidate = valid_refinement();
            candidate.phases.get_mut(&PhaseId::new("leaf").unwrap()).unwrap().effects.effects.insert(EffectId::new("publish").unwrap());
            let mut phases: Vec<_> = candidate.phases.into_values().collect();
            if reverse { phases.reverse(); }
            if let Failure::Widening { field, path, .. } = tree_spec(phases).admit().unwrap_err() {
                failures.insert((field, path.phases().to_vec()));
            }
        }
        assert_eq!(failures.len(), 1);
    }
}
