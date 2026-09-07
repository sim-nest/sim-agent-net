fn revision_datum(
    parent: Option<&RoadmapRevisionId>,
    spec: &RoadmapSpec,
    change: &RevisionChange,
) -> Datum {
    node(
        "RevisionIdentityV2",
        vec![
            (
                "parent",
                parent.map(|value| content(&value.0)).unwrap_or(Datum::Nil),
            ),
            ("roadmap", spec_datum(spec)),
            (
                "change",
                node(
                    "RevisionChangeV1",
                    vec![
                        ("id", text(&change.id)),
                        ("rationale", Datum::String(change.rationale.clone())),
                    ],
                ),
            ),
        ],
    )
}

fn spec_datum(spec: &RoadmapSpec) -> Datum {
    node(
        "RoadmapSpecV2",
        vec![
            ("schema", text(&spec.schema)),
            ("id", text(&spec.id)),
            (
                "charter",
                node(
                    "CharterV1",
                    vec![
                        ("title", Datum::String(spec.charter.title.clone())),
                        ("intent", Datum::String(spec.charter.intent.clone())),
                    ],
                ),
            ),
            ("root", text(&spec.root)),
            (
                "imports",
                Datum::Map(
                    spec.imports
                        .iter()
                        .map(|(id, pin)| (text(id), pinned_roadmap_datum(pin)))
                        .collect(),
                ),
            ),
            (
                "phases",
                Datum::Map(
                    spec.phases
                        .iter()
                        .map(|(id, phase)| (text(id), phase_datum(phase)))
                        .collect(),
                ),
            ),
            ("limits", limits_datum(spec.limits)),
        ],
    )
}

fn phase_datum(phase: &PhaseSpec) -> Datum {
    node(
        "PhaseSpecV2",
        vec![
            ("id", text(&phase.id)),
            (
                "parent",
                phase.parent.as_ref().map(text).unwrap_or(Datum::Nil),
            ),
            ("title", Datum::String(phase.title.clone())),
            ("intent", Datum::String(phase.intent.clone())),
            ("body", phase_body_datum(&phase.body)),
            (
                "dependencies",
                Datum::List(
                    phase
                        .dependencies
                        .iter()
                        .map(phase_dependency_datum)
                        .collect(),
                ),
            ),
            ("owners", owner_envelope_datum(&phase.owners)),
            ("resources", resource_envelope_datum(&phase.resources)),
            ("effects", effect_envelope_datum(&phase.effects)),
            (
                "capabilities",
                capability_envelope_datum(&phase.capabilities),
            ),
            ("changes", change_envelope_datum(&phase.changes)),
            ("acceptance", acceptance_datum(&phase.acceptance)),
            (
                "coverage",
                Datum::List(
                    phase
                        .coverage
                        .iter()
                        .map(obligation_coverage_datum)
                        .collect(),
                ),
            ),
            (
                "outputs",
                Datum::Map(
                    phase
                        .outputs
                        .iter()
                        .map(|(id, output)| (text(id), output_contract_datum(output)))
                        .collect(),
                ),
            ),
            ("guide", implementation_guide_datum(&phase.guide)),
            ("origin", phase_origin_datum(&phase.origin)),
        ],
    )
}

fn phase_body_datum(body: &PhaseBody) -> Datum {
    match body {
        PhaseBody::Leaf { checkpoints } => node(
            "LeafBodyV1",
            vec![(
                "checkpoints",
                Datum::List(
                    checkpoints
                        .iter()
                        .map(|checkpoint| {
                            node(
                                "CheckpointV1",
                                vec![
                                    ("id", text(&checkpoint.id)),
                                    (
                                        "statement",
                                        Datum::String(checkpoint.statement.clone()),
                                    ),
                                ],
                            )
                        })
                        .collect(),
                ),
            )],
        ),
        PhaseBody::Composite { children } => node(
            "CompositeBodyV1",
            vec![("children", Datum::List(children.iter().map(text).collect()))],
        ),
    }
}

fn phase_dependency_datum(dependency: &PhaseDependency) -> Datum {
    match dependency {
        PhaseDependency::Requires(phase) => {
            node("RequiresV1", vec![("phase", phase_ref_datum(phase))])
        }
        PhaseDependency::Consumes(output) => {
            node("ConsumesV1", vec![("output", output_ref_datum(output))])
        }
        PhaseDependency::PrefersAfter(phase) => {
            node("PrefersAfterV1", vec![("phase", phase_ref_datum(phase))])
        }
    }
}

fn phase_ref_datum(reference: &PhaseRef) -> Datum {
    match reference {
        PhaseRef::Local(phase) => node("LocalPhaseV1", vec![("phase", text(phase))]),
        PhaseRef::Imported {
            import,
            phase,
            phase_content,
        } => node(
            "ImportedPhaseV1",
            vec![
                ("import", text(import)),
                ("phase", text(phase)),
                ("content", content(phase_content)),
            ],
        ),
    }
}

fn output_ref_datum(output: &OutputRef) -> Datum {
    node(
        "OutputRefV1",
        vec![
            ("phase", phase_ref_datum(&output.phase)),
            ("output", text(&output.output)),
        ],
    )
}

fn owner_envelope_datum(envelope: &OwnerEnvelope) -> Datum {
    node(
        "OwnerEnvelopeV1",
        vec![
            ("mutable", string_set(&envelope.mutable)),
            ("read-only", string_set(&envelope.read_only)),
        ],
    )
}

fn resource_envelope_datum(envelope: &ResourceEnvelope) -> Datum {
    node(
        "ResourceEnvelopeV1",
        vec![("resources", string_set(&envelope.resources))],
    )
}

fn effect_envelope_datum(envelope: &EffectEnvelope) -> Datum {
    node(
        "EffectEnvelopeV1",
        vec![("effects", string_set(&envelope.effects))],
    )
}

fn capability_envelope_datum(envelope: &CapabilityEnvelope) -> Datum {
    node(
        "CapabilityEnvelopeV1",
        vec![("capabilities", string_set(&envelope.capabilities))],
    )
}

fn change_envelope_datum(envelope: &ChangeEnvelope) -> Datum {
    node(
        "ChangeEnvelopeV1",
        vec![("targets", string_set(&envelope.targets))],
    )
}

fn acceptance_datum(acceptance: &AcceptanceContract) -> Datum {
    node(
        "AcceptanceContractV1",
        vec![
            (
                "policy",
                Datum::Symbol(tag(match acceptance.policy {
                    ProofPolicy::All => "all",
                    ProofPolicy::Any => "any",
                })),
            ),
            (
                "statements",
                Datum::Map(
                    acceptance
                        .statements
                        .iter()
                        .map(|(id, statement)| (text(id), acceptance_statement_datum(statement)))
                        .collect(),
                ),
            ),
        ],
    )
}

fn acceptance_statement_datum(statement: &AcceptanceStatement) -> Datum {
    node(
        "AcceptanceStatementV1",
        vec![
            ("obligation", text(&statement.obligation)),
            ("subject", reference_datum(&statement.subject)),
            (
                "predicate",
                Datum::Symbol(statement.predicate.clone()),
            ),
            ("object", reference_datum(&statement.object)),
            (
                "supporting-refs",
                Datum::Set(
                    statement
                        .supporting_refs
                        .iter()
                        .map(reference_datum)
                        .collect(),
                ),
            ),
        ],
    )
}

fn reference_datum(reference: &Ref) -> Datum {
    match reference {
        Ref::Symbol(symbol) => node(
            "SymbolRefV1",
            vec![("symbol", Datum::Symbol(symbol.clone()))],
        ),
        Ref::Content(id) => node("ContentRefV1", vec![("content", content(id))]),
        Ref::Handle(id) => node(
            "HandleRefV1",
            vec![("handle", unsigned("u128", id.0.to_string()))],
        ),
        Ref::Coord(coordinate) => node(
            "CoordinateRefV1",
            vec![
                ("space", Datum::Symbol(coordinate.space.clone())),
                ("ordinal", content(&coordinate.ordinal)),
            ],
        ),
    }
}

fn obligation_coverage_datum(coverage: &ObligationCoverage) -> Datum {
    match coverage {
        ObligationCoverage::Contributes {
            parent,
            phase,
            child,
        } => node(
            "ContributesV1",
            vec![
                ("parent", text(parent)),
                ("phase", text(phase)),
                ("child", text(child)),
            ],
        ),
        ObligationCoverage::RetainedAtParent { parent } => {
            node("RetainedAtParentV1", vec![("parent", text(parent))])
        }
    }
}

fn output_contract_datum(output: &OutputContract) -> Datum {
    node(
        "OutputContractV1",
        vec![
            ("description", Datum::String(output.description.clone())),
            (
                "content",
                output.content.as_ref().map(content).unwrap_or(Datum::Nil),
            ),
        ],
    )
}

fn pinned_roadmap_datum(pin: &PinnedRoadmapRef) -> Datum {
    node(
        "PinnedRoadmapRefV1",
        vec![
            ("roadmap", text(&pin.roadmap)),
            ("revision", content(&pin.revision.0)),
            ("root-phase", text(&pin.root_phase)),
            ("root-content", content(&pin.root_content)),
        ],
    )
}

fn phase_origin_datum(origin: &PhaseOrigin) -> Datum {
    match origin {
        PhaseOrigin::Authored => Datum::Symbol(tag("authored")),
        PhaseOrigin::Imported {
            import,
            phase,
            phase_content,
        } => node(
            "ImportedOriginV1",
            vec![
                ("import", text(import)),
                ("phase", text(phase)),
                ("content", content(phase_content)),
            ],
        ),
    }
}


fn limits_datum(limits: Limits) -> Datum {
    node(
        "LimitsV1",
        vec![
            ("id-bytes", usize_datum(limits.id_bytes)),
            ("document-bytes", usize_datum(limits.document_bytes)),
            ("prose-bytes", usize_datum(limits.prose_bytes)),
            ("phases", usize_datum(limits.phases)),
            ("imports", usize_datum(limits.imports)),
            (
                "outputs-per-phase",
                usize_datum(limits.outputs_per_phase),
            ),
            (
                "checkpoints-per-phase",
                usize_datum(limits.checkpoints_per_phase),
            ),
            (
                "children-per-phase",
                usize_datum(limits.children_per_phase),
            ),
            ("tree-depth", usize_datum(limits.tree_depth)),
            ("causal-path", usize_datum(limits.causal_path)),
            ("guide-queries", usize_datum(limits.guide_queries)),
            ("guide-targets", usize_datum(limits.guide_targets)),
            ("guide-promises", usize_datum(limits.guide_promises)),
            ("guide-sketches", usize_datum(limits.guide_sketches)),
            ("sketch-bytes", usize_datum(limits.sketch_bytes)),
            ("sketch-bindings", usize_datum(limits.sketch_bindings)),
        ],
    )
}

fn content(id: &ContentId) -> Datum {
    node(
        "ContentIdV1",
        vec![
            ("algorithm", Datum::Symbol(id.algorithm.clone())),
            ("bytes", Datum::Bytes(id.bytes.to_vec())),
        ],
    )
}

fn string_set<T: ToString>(values: &BTreeSet<T>) -> Datum {
    Datum::Set(values.iter().map(text).collect())
}

fn text(value: &impl ToString) -> Datum {
    Datum::String(value.to_string())
}

fn usize_datum(value: usize) -> Datum {
    unsigned("usize", value.to_string())
}

fn unsigned(domain: &str, canonical: String) -> Datum {
    Datum::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", domain),
        canonical,
    })
}

fn node(name: &str, fields: Vec<(&str, Datum)>) -> Datum {
    Datum::Node {
        tag: tag(name),
        fields: fields
            .into_iter()
            .map(|(name, value)| (Symbol::new(name), value))
            .collect(),
    }
}

fn tag(name: &str) -> Symbol {
    Symbol::qualified("roadmap", name)
}
