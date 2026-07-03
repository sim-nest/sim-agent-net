use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs};
use crate::AgentPattern;
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_kernel::{Args, EncodeOptions, Expr, Lib, ReadPolicy, Symbol};

#[test]
fn agent_pattern_constructor_exposes_card_metadata() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();

    let pattern = pattern_value(&mut cx);
    let descriptor = pattern.object().downcast_ref::<AgentPattern>().unwrap();
    assert_eq!(descriptor.id, Symbol::new("a30-001-autonomous-decision"));
    assert_eq!(descriptor.sense[0].name, Symbol::new("input"));

    let expr = pattern.object().as_expr(&mut cx).unwrap();
    assert_eq!(
        field_symbol(&expr, "kind"),
        Some(Symbol::new("agent-pattern"))
    );
    let card = field(&expr, "card").unwrap();
    assert_eq!(
        field_symbol(card, "kind"),
        Some(Symbol::new("agent-pattern"))
    );
    assert_eq!(
        list_symbols(field(card, "fields").unwrap()),
        vec![
            "sense",
            "model",
            "plan",
            "act",
            "memory",
            "tools",
            "policy",
            "evaluation",
            "guardrail",
            "trace",
        ]
    );
}

#[test]
fn agent_pattern_round_trips_through_lisp_and_json_codecs() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let pattern = pattern_value(&mut cx);
    let expr = pattern.object().as_expr(&mut cx).unwrap();
    for codec in [
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "lisp"),
    ] {
        let encoded = encode_with_codec(&mut cx, &codec, &expr, EncodeOptions::default()).unwrap();
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
        assert!(decoded.canonical_eq(&expr));
    }
}

#[test]
fn agent_pattern_function_is_exported() {
    let exports = crate::AgentLib.manifest().exports;
    assert!(exports.iter().any(|export| {
        matches!(
            export,
            sim_kernel::Export::Function { symbol, .. }
                if *symbol == Symbol::qualified("agent", "pattern")
        )
    }));
}

fn pattern_value(cx: &mut sim_kernel::Cx) -> sim_kernel::Value {
    let args = vec![
        symbol_value(cx, ":id"),
        symbol_value(cx, "a30-001-autonomous-decision"),
        symbol_value(cx, ":description"),
        cx.factory()
            .string("deterministic autonomous decision descriptor".to_owned())
            .unwrap(),
        symbol_value(cx, ":sense"),
        slot_list(cx, &["input", "policy"]),
        symbol_value(cx, ":model"),
        slot_list(cx, &["fake-runner"]),
        symbol_value(cx, ":plan"),
        slot_list(cx, &["threshold-check"]),
        symbol_value(cx, ":act"),
        slot_list(cx, &["approve-ticket"]),
        symbol_value(cx, ":memory"),
        slot_list(cx, &["none"]),
        symbol_value(cx, ":tools"),
        slot_list(cx, &["none"]),
        symbol_value(cx, ":policy"),
        slot_list(cx, &["low-risk-threshold"]),
        symbol_value(cx, ":evaluation"),
        slot_list(cx, &["deterministic-match"]),
        symbol_value(cx, ":guardrail"),
        slot_list(cx, &["offline-synthetic-fixture"]),
        symbol_value(cx, ":trace"),
        slot_list(cx, &["decision-record"]),
    ];
    cx.call_function(&Symbol::qualified("agent", "pattern"), Args::new(args))
        .unwrap()
}

fn symbol_value(cx: &mut sim_kernel::Cx, name: &str) -> sim_kernel::Value {
    cx.factory().symbol(Symbol::new(name)).unwrap()
}

fn slot_list(cx: &mut sim_kernel::Cx, names: &[&str]) -> sim_kernel::Value {
    let values = names
        .iter()
        .map(|name| symbol_value(cx, name))
        .collect::<Vec<_>>();
    cx.factory().list(values).unwrap()
}

fn field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| {
        matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == name).then_some(value)
    })
}

fn field_symbol(expr: &Expr, name: &str) -> Option<Symbol> {
    match field(expr, name) {
        Some(Expr::Symbol(symbol)) => Some(symbol.clone()),
        _ => None,
    }
}

fn list_symbols(expr: &Expr) -> Vec<&str> {
    let Expr::List(items) = expr else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Expr::Symbol(symbol) => Some(symbol.name.as_ref()),
            _ => None,
        })
        .collect()
}
