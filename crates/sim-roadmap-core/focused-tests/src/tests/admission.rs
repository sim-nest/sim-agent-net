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
