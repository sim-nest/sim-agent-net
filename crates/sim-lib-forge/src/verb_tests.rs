use sim_kernel::{Args, Cx, Export, Expr, Lib, Symbol, testing::bare_cx};

use crate::{ForgeLib, forge_entrypoint_symbol, forge_verb};

fn payload(args: &[&str]) -> Expr {
    Expr::List(
        args.iter()
            .copied()
            .map(str::to_owned)
            .map(Expr::String)
            .collect(),
    )
}

fn map_field<'a>(expr: &'a Expr, name: &str) -> &'a Expr {
    let Expr::Map(entries) = expr else {
        panic!("report is not a map: {expr:?}");
    };
    let key = Expr::Symbol(Symbol::new(name));
    entries
        .iter()
        .find_map(|(entry_key, value)| (entry_key == &key).then_some(value))
        .unwrap_or_else(|| panic!("missing report field {name}"))
}

fn number_field<'a>(expr: &'a Expr, name: &str) -> &'a str {
    match map_field(expr, name) {
        Expr::Number(number) => number.canonical.as_str(),
        other => panic!("{name} is not a number: {other:?}"),
    }
}

fn bool_field(expr: &Expr, name: &str) -> bool {
    match map_field(expr, name) {
        Expr::Bool(value) => *value,
        other => panic!("{name} is not a bool: {other:?}"),
    }
}

fn string_field<'a>(expr: &'a Expr, name: &str) -> &'a str {
    match map_field(expr, name) {
        Expr::String(value) => value,
        other => panic!("{name} is not a string: {other:?}"),
    }
}

fn symbol_field(expr: &Expr, name: &str) -> Symbol {
    match map_field(expr, name) {
        Expr::Symbol(symbol) => symbol.clone(),
        other => panic!("{name} is not a symbol: {other:?}"),
    }
}

fn cli_envelope(cx: &mut Cx, args: &[&str]) -> sim_kernel::Value {
    let arg_values = args
        .iter()
        .copied()
        .map(|arg| cx.factory().string(arg.to_owned()).unwrap())
        .collect::<Vec<_>>();
    let arg_values = cx.factory().list(arg_values).unwrap();
    cx.factory()
        .table(vec![
            (
                Symbol::new("verb"),
                cx.factory().string("forge".to_owned()).unwrap(),
            ),
            (Symbol::new("args"), arg_values),
        ])
        .unwrap()
}

#[test]
fn forge_lib_exports_cli_entrypoint() {
    let lib = ForgeLib;
    let manifest = lib.manifest();

    assert_eq!(manifest.id, Symbol::qualified("sim", "forge"));
    assert!(manifest.exports.contains(&Export::Function {
        symbol: forge_entrypoint_symbol(),
        function_id: None,
    }));
}

#[test]
fn forge_loaded_entrypoint_accepts_cli_envelope() {
    let mut cx = bare_cx();
    let lib = ForgeLib;
    cx.load_lib(&lib).unwrap();
    let envelope = cli_envelope(&mut cx, &["forge", "show", "summarize-contract"]);

    let result = cx
        .call_function(&forge_entrypoint_symbol(), Args::new(vec![envelope]))
        .unwrap();

    assert!(result.object().truth(&mut cx).unwrap());
}

#[test]
fn run_on_golden_skips_lift_call() {
    let mut cx = bare_cx();

    let report = forge_verb(
        &mut cx,
        &payload(&["forge", "run", "summarize-contract", "input.json"]),
    )
    .unwrap();

    assert_eq!(
        symbol_field(&report, "status"),
        Symbol::qualified("forge", "golden")
    );
    assert_eq!(number_field(&report, "compiler-calls"), "0");
    assert!(bool_field(&report, "artifact-cache-hit"));
}

#[test]
fn identical_replayed_run_makes_no_execution_call() {
    let mut cx = bare_cx();

    let report = forge_verb(
        &mut cx,
        &payload(&["forge", "run", "summarize-contract", "input.json"]),
    )
    .unwrap();

    assert_eq!(number_field(&report, "execution-calls"), "0");
    assert_eq!(number_field(&report, "replay-hits"), "1");
    assert!(bool_field(&report, "answer-replay-hit"));
}

#[test]
fn lift_surfaces_inferred_shape_for_approval() {
    let mut cx = bare_cx();

    let report = forge_verb(
        &mut cx,
        &payload(&["forge", "lift", "summarize the contract and flag risks"]),
    )
    .unwrap();

    assert_eq!(
        symbol_field(&report, "status"),
        Symbol::qualified("forge", "candidate")
    );
    assert!(string_field(&report, "inferred-return-shape").contains("Shape Summary"));
    assert_eq!(
        string_field(&report, "review-surface"),
        "surface://forge/summarize-contract"
    );
    assert!(matches!(map_field(&report, "review-fields"), Expr::List(fields) if fields.len() == 2));
}
