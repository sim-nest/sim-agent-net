//! Integration tests for the cookbook runtime ops against a real eval-capable
//! `Cx` (core class stubs plus the Lisp codec).

use std::sync::{Arc, Mutex};

use sim_codec_lisp::LispCodecLib;
use sim_cookbook::RecipeStore;
use sim_kernel::{Args, Callable, Cx, Error, Expr, Symbol, Value, read_eval_capability};
use sim_test_support::core_cx;

use crate::install_cookbook_lib;
use crate::ops::{CookbookOp, OpKind};
use crate::run::run_recipe;
#[cfg(feature = "seed-recipes")]
use crate::{install_seeded_cookbook_lib, seeded_recipe_store};

fn setup_cx() -> Cx {
    let mut cx = core_cx();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    cx
}

// Book id "lisp" so the recipe's defaulted `requires = ["lisp"]` is satisfied
// by the loaded `codec:lisp` lib (matched by its unqualified tail).
fn lisp_book() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("book.toml", b"book = \"lisp\"\ntitle = \"Lisp\"\n" as &[u8]),
        (
            "01-basics/quote/recipe.toml",
            b"id = \"quote\"\ntitle = \"Quote\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\n[[expect]]\nform = 0\nresult = \"ok\"\n",
        ),
        ("01-basics/quote/s", b"(quote ok)"),
        ("01-basics/quote/p", b"quote a symbol"),
    ]
}

fn store_with(book: &[(&str, &[u8])]) -> RecipeStore {
    let mut store = RecipeStore::new();
    store.register_book(book).unwrap();
    store
}

#[test]
fn run_executes_and_checks_expectation() {
    let mut cx = setup_cx();
    let card = store_with(&lisp_book())
        .card("lisp/01-basics/quote")
        .cloned()
        .unwrap();
    cx.grant(read_eval_capability());
    let run = run_recipe(&mut cx, &card).unwrap();
    assert!(run.ok, "expected pass, got {run:?}");
    assert_eq!(run.results, ["ok"]);
    assert!(run.checks[0].pass);
}

#[test]
fn run_reports_failing_expectation() {
    let mut cx = setup_cx();
    let book: Vec<(&str, &[u8])> = vec![
        ("book.toml", b"book = \"lisp\"\ntitle = \"Lisp\"\n"),
        (
            "01-basics/quote/recipe.toml",
            b"id = \"quote\"\ntitle = \"Quote\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\n[[expect]]\nform = 0\nresult = \"nope\"\n",
        ),
        ("01-basics/quote/s", b"(quote ok)"),
        ("01-basics/quote/p", b"quote a symbol"),
    ];
    let card = store_with(&book)
        .card("lisp/01-basics/quote")
        .cloned()
        .unwrap();
    cx.grant(read_eval_capability());
    let run = run_recipe(&mut cx, &card).unwrap();
    assert!(!run.ok);
    assert!(!run.checks[0].pass);
    assert_eq!(run.checks[0].actual, "ok");
}

#[test]
fn run_errors_on_missing_requires() {
    let mut cx = setup_cx();
    let book: Vec<(&str, &[u8])> = vec![
        ("book.toml", b"book = \"lisp\"\ntitle = \"Lisp\"\n"),
        (
            "c/r/recipe.toml",
            b"id = \"r\"\ntitle = \"R\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\nrequires = [\"ghost-lib\"]\n",
        ),
        ("c/r/s", b"(quote ok)"),
        ("c/r/p", b"x"),
    ];
    let card = store_with(&book).card("lisp/c/r").cloned().unwrap();
    let err = run_recipe(&mut cx, &card).unwrap_err();
    match err {
        Error::Eval(message) => assert!(message.contains("requires libs not loaded"), "{message}"),
        other => panic!("expected Eval error, got {other:?}"),
    }
}

#[test]
fn run_op_requires_eval_capability() {
    let mut cx = setup_cx();
    let store = store_with(&lisp_book());
    let op = CookbookOp::new(Arc::new(Mutex::new(store)), OpKind::Run);
    let id = cx
        .factory()
        .string("lisp/01-basics/quote".to_string())
        .unwrap();
    let err = op.call(&mut cx, Args::new(vec![id])).unwrap_err();
    assert!(matches!(err, Error::CapabilityDenied { .. }), "{err:?}");
}

#[test]
fn books_op_returns_a_value() {
    let mut cx = setup_cx();
    let store = store_with(&lisp_book());
    let op = CookbookOp::new(Arc::new(Mutex::new(store)), OpKind::Books);
    assert!(op.call(&mut cx, Args::new(Vec::new())).is_ok());
}

#[test]
fn cookbook_lib_claims_loaded_cli_cookbook_entrypoint() {
    let mut cx = setup_cx();
    install_cookbook_lib(&mut cx, store_with(&lisp_book())).unwrap();
    let symbol = Symbol::qualified("cli", "main/cookbook");
    let envelope = cli_envelope(&mut cx, "cookbook", &["cookbook", "list"]);

    let value = cx
        .call_function(&symbol, Args::new(vec![envelope]))
        .unwrap();

    assert!(value.object().truth(&mut cx).unwrap());
}

#[cfg(feature = "seed-recipes")]
#[test]
fn seeded_store_loads_real_embedded_books() {
    let store = seeded_recipe_store().unwrap();
    assert!(store.len() >= 8, "expected seed recipes, got {store:?}");
    assert!(store.card("codec/lisp/01-basics/quote-symbol").is_some());
}

fn cli_envelope(cx: &mut Cx, verb: &str, args: &[&str]) -> Value {
    let verb = cx.factory().string(verb.to_owned()).unwrap();
    let args = cx
        .factory()
        .list(
            args.iter()
                .map(|arg| cx.factory().string((*arg).to_owned()).unwrap())
                .collect(),
        )
        .unwrap();
    cx.factory()
        .table(vec![
            (Symbol::new("verb"), verb),
            (Symbol::new("args"), args),
        ])
        .unwrap()
}

#[cfg(feature = "seed-recipes")]
#[test]
fn seeded_cookbook_list_is_non_empty() {
    let mut cx = setup_cx();
    install_seeded_cookbook_lib(&mut cx).unwrap();
    let value = cx
        .call_function(
            &Symbol::qualified("cookbook", "list"),
            Args::new(Vec::new()),
        )
        .unwrap();
    let Expr::List(items) = value.object().as_expr(&mut cx).unwrap() else {
        panic!("cookbook:list should return a list");
    };
    assert!(!items.is_empty());
}

#[cfg(feature = "seed-recipes")]
#[test]
fn seeded_expectation_recipe_runs_green() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    let card = seeded_recipe_store()
        .unwrap()
        .card("codec/lisp/01-basics/quote-symbol")
        .cloned()
        .unwrap();
    let run = run_recipe(&mut cx, &card).unwrap();
    assert!(run.ok, "expected seeded run to pass, got {run:?}");
    assert_eq!(run.results, ["codec-lisp-ok"]);
}
