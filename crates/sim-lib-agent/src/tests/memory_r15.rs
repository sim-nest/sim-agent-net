use super::support::{
    eval_cx, install_agent_lib, install_roundtrip_codecs, install_test_codec, temp_memory_path,
};
use crate::{
    AgentLib, Component, FILE_WRITE_CAPABILITY, MemoryBackend, MemoryKind, PersonaMemory,
    VectorMemory, WorkingMemory,
};
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_kernel::{EncodeOptions, Expr, Lib, ReadPolicy, Symbol};

#[test]
fn vector_memory_ranks_by_cosine_and_persists_messages() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let memory = VectorMemory::open(None, vec![Symbol::qualified("codec", "binary")]).unwrap();
    let first = cx.factory().string("SIM codec framing".to_owned()).unwrap();
    memory.append(&mut cx, first).unwrap();
    let second = cx.factory().string("garden tomatoes".to_owned()).unwrap();
    memory.append(&mut cx, second).unwrap();
    let third = cx
        .factory()
        .string("SIM codec registry".to_owned())
        .unwrap();
    memory.append(&mut cx, third).unwrap();

    let search = memory
        .search(
            &mut cx,
            Expr::List(vec![
                Expr::Symbol(Symbol::new("query")),
                Expr::String("SIM registry".to_owned()),
            ]),
            2,
        )
        .unwrap();
    let ranked = search
        .into_iter()
        .map(|value| value.object().as_expr(&mut cx).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        ranked,
        vec![
            Expr::String("SIM codec registry".to_owned()),
            Expr::String("SIM codec framing".to_owned()),
        ]
    );

    cx.grant_named(FILE_WRITE_CAPABILITY);
    let path = temp_memory_path("vector");
    let durable = VectorMemory::open(
        Some(path.clone()),
        vec![Symbol::qualified("codec", "binary")],
    )
    .unwrap();
    let durable_note = cx
        .factory()
        .string("durable vector note".to_owned())
        .unwrap();
    durable.append(&mut cx, durable_note).unwrap();
    let reopened = VectorMemory::open(
        Some(path.clone()),
        vec![Symbol::qualified("codec", "binary")],
    )
    .unwrap();
    assert_eq!(
        reopened.snapshot(&mut cx).unwrap(),
        Expr::List(vec![Expr::String("durable vector note".to_owned())])
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn persona_memory_holds_state_and_notes() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let memory = PersonaMemory::open(None, vec![Symbol::qualified("codec", "binary")]).unwrap();
    memory
        .restore(
            &mut cx,
            Expr::List(vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("state")),
                    Expr::Map(vec![(
                        Expr::Symbol(Symbol::new("tone")),
                        Expr::String("terse".to_owned()),
                    )]),
                ),
                (
                    Expr::Symbol(Symbol::new("notes")),
                    Expr::List(vec![Expr::String("prefers concise answers".to_owned())]),
                ),
            ])]),
        )
        .unwrap();
    let note = cx
        .factory()
        .string("remember to cite codec choices".to_owned())
        .unwrap();
    memory.append(&mut cx, note).unwrap();

    let recent = memory.recent(&mut cx, 2).unwrap();
    let recent = recent
        .into_iter()
        .map(|value| value.object().as_expr(&mut cx).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        recent,
        vec![
            Expr::String("prefers concise answers".to_owned()),
            Expr::String("remember to cite codec choices".to_owned()),
        ]
    );

    let snapshot = memory.snapshot(&mut cx).unwrap();
    let Expr::List(ref items) = snapshot else {
        panic!("persona snapshot should be a list");
    };
    let Expr::Map(entries) = &items[0] else {
        panic!("persona snapshot should wrap a table");
    };
    assert!(entries.iter().any(|(key, value)| {
        *key == Expr::Symbol(Symbol::new("state"))
            && *value
                == Expr::Map(vec![(
                    Expr::Symbol(Symbol::new("tone")),
                    Expr::String("terse".to_owned()),
                )])
    }));

    cx.grant_named(FILE_WRITE_CAPABILITY);
    let path = temp_memory_path("persona");
    let durable = PersonaMemory::open(
        Some(path.clone()),
        vec![Symbol::qualified("codec", "binary")],
    )
    .unwrap();
    durable.restore(&mut cx, Expr::List(items.clone())).unwrap();
    let reopened = PersonaMemory::open(
        Some(path.clone()),
        vec![Symbol::qualified("codec", "binary")],
    )
    .unwrap();
    assert_eq!(reopened.snapshot(&mut cx).unwrap(), snapshot);
    let _ = std::fs::remove_file(path);
}

#[test]
fn memory_snapshot_round_trips_through_binary_json_and_lisp_codecs() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let working = WorkingMemory::new(vec![
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "lisp"),
    ]);
    working
        .restore(
            &mut cx,
            Expr::List(vec![
                Expr::Map(vec![(
                    Expr::Symbol(Symbol::new("speaker")),
                    Expr::String("planner".to_owned()),
                )]),
                Expr::String("SIM codecs".to_owned()),
            ]),
        )
        .unwrap();
    let vector = VectorMemory::open(
        None,
        vec![
            Symbol::qualified("codec", "binary"),
            Symbol::qualified("codec", "json"),
            Symbol::qualified("codec", "lisp"),
        ],
    )
    .unwrap();
    vector
        .restore(
            &mut cx,
            Expr::List(vec![
                Expr::String("SIM codecs".to_owned()),
                Expr::String("vector memory".to_owned()),
            ]),
        )
        .unwrap();
    let persona = PersonaMemory::open(
        None,
        vec![
            Symbol::qualified("codec", "binary"),
            Symbol::qualified("codec", "json"),
            Symbol::qualified("codec", "lisp"),
        ],
    )
    .unwrap();
    persona
        .restore(
            &mut cx,
            Expr::List(vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("state")),
                    Expr::Map(vec![(
                        Expr::Symbol(Symbol::new("language")),
                        Expr::String("en".to_owned()),
                    )]),
                ),
                (
                    Expr::Symbol(Symbol::new("notes")),
                    Expr::List(vec![Expr::String("prefer terse output".to_owned())]),
                ),
            ])]),
        )
        .unwrap();

    for snapshot in [
        working.snapshot(&mut cx).unwrap(),
        vector.snapshot(&mut cx).unwrap(),
        persona.snapshot(&mut cx).unwrap(),
    ] {
        for codec in [
            Symbol::qualified("codec", "binary"),
            Symbol::qualified("codec", "json"),
            Symbol::qualified("codec", "lisp"),
        ] {
            let encoded =
                encode_with_codec(&mut cx, &codec, &snapshot, EncodeOptions::default()).unwrap();
            let decoded = decode_with_codec(
                &mut cx,
                &codec,
                match encoded {
                    sim_codec::Output::Text(text) => Input::Text(text),
                    sim_codec::Output::Bytes(bytes) => Input::Bytes(bytes),
                },
                ReadPolicy::default(),
            )
            .unwrap();
            assert!(decoded.canonical_eq(&snapshot));
        }
    }
}

#[test]
fn memory_metadata_reports_component_kind() {
    let mut cx = eval_cx();
    let memory = WorkingMemory::new(Vec::new());
    assert_eq!(memory.kind(), crate::ComponentKind::Memory);
    assert_eq!(
        MemoryKind::Blackboard.as_symbol(),
        Symbol::new("blackboard")
    );
    assert_eq!(MemoryKind::Vector.as_symbol(), Symbol::new("vector"));
    assert_eq!(MemoryKind::Persona.as_symbol(), Symbol::new("persona"));
    let Expr::Map(entries) = memory.reflect(&mut cx).unwrap() else {
        panic!("memory reflect should produce a table expr");
    };
    assert!(entries.iter().any(|(key, value)| {
        *key == Expr::Symbol(Symbol::new("kind")) && *value == Expr::Symbol(Symbol::new("working"))
    }));
}

#[test]
fn memory_function_registration_lists_full_memory_set() {
    let exports = AgentLib.manifest().exports;
    for symbol in [
        Symbol::qualified("memory", "working"),
        Symbol::qualified("memory", "file"),
        Symbol::qualified("memory", "vector"),
        Symbol::qualified("memory", "blackboard"),
        Symbol::qualified("memory", "persona"),
    ] {
        assert!(exports.iter().any(|export| {
            matches!(export, sim_kernel::Export::Function { symbol: export_symbol, .. } if *export_symbol == symbol)
        }));
    }
}
