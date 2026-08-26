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
                        &p.capabilities,
                        &p.changes,
                        &p.acceptance,
                        &p.coverage,
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
