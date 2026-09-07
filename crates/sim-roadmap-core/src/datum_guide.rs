use super::*;

pub(super) fn implementation_guide_datum(guide: &ImplementationGuide) -> Datum {
    node(
        "ImplementationGuideV1",
        vec![
            (
                "uses",
                Datum::List(guide.uses.iter().map(source_query_datum).collect()),
            ),
            (
                "change-targets",
                Datum::List(
                    guide
                        .change_targets
                        .iter()
                        .map(change_target_datum)
                        .collect(),
                ),
            ),
            (
                "promises",
                Datum::List(guide.promises.iter().map(promise_datum).collect()),
            ),
            (
                "sketches",
                Datum::List(guide.sketches.iter().map(sketch_datum).collect()),
            ),
        ],
    )
}

fn source_query_datum(query: &SourceQuery) -> Datum {
    match query {
        SourceQuery::Anchor(id) => node("AnchorQueryV1", vec![("id", Datum::String(id.clone()))]),
        SourceQuery::Excerpt(id) => node("ExcerptQueryV1", vec![("id", Datum::String(id.clone()))]),
        SourceQuery::Specimen(id) => {
            node("SpecimenQueryV1", vec![("id", Datum::String(id.clone()))])
        }
    }
}

fn change_target_datum(target: &ChangeTarget) -> Datum {
    node(
        "ChangeTargetV1",
        vec![
            ("change", text(&target.change)),
            ("owner", text(&target.owner)),
            (
                "package",
                target
                    .package
                    .as_ref()
                    .map(|value| Datum::String(value.clone()))
                    .unwrap_or(Datum::Nil),
            ),
            ("description", Datum::String(target.description.clone())),
        ],
    )
}

fn promise_datum(promise: &Promise) -> Datum {
    match promise {
        Promise::PublicDeclaration { id, owner, anchor } => node(
            "PublicDeclarationPromiseV1",
            vec![
                ("id", text(id)),
                ("owner", text(owner)),
                ("anchor", Datum::String(anchor.clone())),
            ],
        ),
        Promise::SourcePostimage {
            id,
            owner,
            path,
            content: id_content,
        } => node(
            "SourcePostimagePromiseV1",
            vec![
                ("id", text(id)),
                ("owner", text(owner)),
                ("path", Datum::String(path.clone())),
                ("content", content(id_content)),
            ],
        ),
        Promise::CheckedSpecimen {
            id,
            owner,
            specimen,
        } => node(
            "CheckedSpecimenPromiseV1",
            vec![
                ("id", text(id)),
                ("owner", text(owner)),
                ("specimen", Datum::String(specimen.clone())),
            ],
        ),
        Promise::ProducedOutput { id, output } => node(
            "ProducedOutputPromiseV1",
            vec![("id", text(id)), ("output", text(output))],
        ),
        Promise::Acceptance { id, obligation } => node(
            "AcceptancePromiseV1",
            vec![("id", text(id)), ("obligation", text(obligation))],
        ),
    }
}

fn sketch_datum(sketch: &AnchoredSketch) -> Datum {
    node(
        "AnchoredSketchV1",
        vec![
            ("id", text(&sketch.id)),
            ("language", sketch_language_datum(&sketch.language)),
            (
                "role",
                Datum::Symbol(tag(match sketch.role {
                    SketchRole::Pattern => "pattern",
                    SketchRole::Interface => "interface",
                    SketchRole::Example => "example",
                    SketchRole::Constraint => "constraint",
                })),
            ),
            ("body", Datum::String(sketch.body.clone())),
            (
                "bindings",
                Datum::List(sketch.bindings.iter().map(sketch_binding_datum).collect()),
            ),
        ],
    )
}

fn sketch_language_datum(language: &SketchLanguage) -> Datum {
    match language {
        SketchLanguage::Rust => Datum::Symbol(tag("rust")),
        SketchLanguage::Sim => Datum::Symbol(tag("sim")),
        SketchLanguage::Text => Datum::Symbol(tag("text")),
        SketchLanguage::Other(name) => node(
            "OtherSketchLanguageV1",
            vec![("name", Datum::String(name.clone()))],
        ),
    }
}

fn sketch_binding_datum(binding: &SketchBinding) -> Datum {
    match binding {
        SketchBinding::Uses { label, query } => node(
            "UsesBindingV1",
            vec![
                ("label", Datum::String(label.clone())),
                ("query", source_query_datum(query)),
            ],
        ),
        SketchBinding::Produces { label, promise } => node(
            "ProducesBindingV1",
            vec![
                ("label", Datum::String(label.clone())),
                ("promise", text(promise)),
            ],
        ),
    }
}
