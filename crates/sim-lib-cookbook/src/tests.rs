//! Integration tests for the cookbook runtime ops against a real eval-capable
//! `Cx` (core class stubs plus the Lisp codec).

use std::sync::{Arc, Mutex};

use sim_codec_lisp::LispCodecLib;
use sim_cookbook::RecipeStore;
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, ClassRef, Cx, Error, Export, Expr, Lib,
    LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
    read_construct_capability, read_eval_capability,
};
use sim_lib_core::{SurfacePackLib, SurfacePackSpec};
use sim_shape::{ExprKind, ExprKindShape};
use sim_test_support::core_cx;

use crate::catalog::EmptyCatalog;
use crate::install_cookbook_lib;
use crate::ops::{CookbookOp, OpKind};
use crate::run::{run_recipe, run_recipe_with_catalog_for_shape_test};
#[cfg(feature = "seed-recipes")]
use crate::{
    LoadableLibList, install_seeded_cookbook_lib, projected_recipe_store,
    run_recipe_with_loadable_libs, seeded_recipe_store,
};

const RECIPE_CUSTOM_CAPABILITY: &str = "recipe.custom";

fn setup_cx() -> Cx {
    let mut cx = core_cx();
    let core = SurfacePackLib {
        spec: SurfacePackSpec {
            lib_id: sim_lib_core::manifest_name(),
            values: Vec::new(),
        },
    };
    cx.load_lib(&core).unwrap();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    cx
}

struct CapabilityProbeLib;

struct CapabilityProbeCallable;

impl Object for CapabilityProbeCallable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<capability-probe>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for CapabilityProbeCallable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.resolve_class(&Symbol::qualified("core", "Function"))
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for CapabilityProbeCallable {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        cx.require(&CapabilityName::new(RECIPE_CUSTOM_CAPABILITY))?;
        cx.factory().string("capability-ok".to_owned())
    }
}

impl Lib for CapabilityProbeLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("test", "capability-probe"),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: capability_probe_symbol(),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            capability_probe_symbol(),
            cx.factory().opaque(Arc::new(CapabilityProbeCallable))?,
        )?;
        Ok(())
    }
}

fn capability_probe_symbol() -> Symbol {
    Symbol::qualified("test", "needs-recipe-capability")
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
fn run_reports_descriptor_on_unresolved_require() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
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
        Error::Eval(message) => assert!(
            message.contains("descriptor: requires not in catalog")
                && message.contains("ghost-lib"),
            "{message}"
        ),
        other => panic!("expected Eval error, got {other:?}"),
    }
}

#[test]
fn run_recipe_is_denied_without_read_eval() {
    // The runner gates on read-eval at the lowest level, so a caller that never
    // obtained read-eval cannot drive an eval, even directly.
    let mut cx = setup_cx();
    let book: Vec<(&str, &[u8])> = vec![
        ("book.toml", b"book = \"lisp\"\ntitle = \"Lisp\"\n"),
        (
            "c/r/recipe.toml",
            b"id = \"r\"\ntitle = \"R\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\n",
        ),
        ("c/r/s", b"(quote ok)"),
        ("c/r/p", b"x"),
    ];
    let card = store_with(&book).card("lisp/c/r").cloned().unwrap();
    let err = run_recipe(&mut cx, &card).unwrap_err();
    assert!(
        matches!(&err, Error::CapabilityDenied { capability } if *capability == read_eval_capability()),
        "expected CapabilityDenied, got {err:?}"
    );
}

#[test]
fn run_recipe_denies_missing_declared_capability() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    let book: Vec<(&str, &[u8])> = vec![
        ("book.toml", b"book = \"lisp\"\ntitle = \"Lisp\"\n"),
        (
            "c/r/recipe.toml",
            b"id = \"r\"\ntitle = \"R\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\ntags = [\"requires-capability:recipe.custom\", \"allow-capability:recipe.custom\"]\n",
        ),
        ("c/r/s", b"(quote ok)"),
        ("c/r/p", b"x"),
    ];
    let card = store_with(&book).card("lisp/c/r").cloned().unwrap();
    let err = run_recipe(&mut cx, &card).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::CapabilityDenied { capability }
                if capability.as_str() == RECIPE_CUSTOM_CAPABILITY
        ),
        "expected missing recipe capability, got {err:?}"
    );
}

#[test]
fn run_recipe_diminishes_to_runner_capabilities() {
    let mut cx = setup_cx();
    cx.load_lib(&CapabilityProbeLib).unwrap();
    cx.grant(read_eval_capability());
    let book: Vec<(&str, &[u8])> = vec![
        ("book.toml", b"book = \"lisp\"\ntitle = \"Lisp\"\n"),
        (
            "c/r/recipe.toml",
            b"id = \"r\"\ntitle = \"R\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\ntags = [\"allow-capability:recipe.custom\"]\n",
        ),
        ("c/r/s", b"(test/needs-recipe-capability nil)"),
        ("c/r/p", b"x"),
    ];
    let card = store_with(&book).card("lisp/c/r").cloned().unwrap();
    let err = run_recipe(&mut cx, &card).unwrap_err();
    assert!(
        matches!(
            &err,
            Error::CapabilityDenied { capability }
                if capability.as_str() == RECIPE_CUSTOM_CAPABILITY
        ),
        "expected diminished recipe capability, got {err:?}"
    );
}

#[test]
fn run_recipe_denies_shape_mismatch() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    let card = store_with(&lisp_book())
        .card("lisp/01-basics/quote")
        .cloned()
        .unwrap();
    let err = run_recipe_with_catalog_for_shape_test(
        &mut cx,
        &EmptyCatalog,
        &card,
        Arc::new(ExprKindShape::new(ExprKind::List)),
    )
    .unwrap_err();
    assert!(matches!(err, Error::WrongShape { .. }), "{err:?}");
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
fn seeded_loadable_directory_projects_load_cards() {
    let directory = SeededLibCatalog::loadable_libs();
    assert!(directory.entry("numbers/cas").is_some());
    assert!(directory.resolve("numbers/cas").is_some());
    assert!(directory.resolve("cas").is_some());

    let cx = setup_cx();
    let store = projected_recipe_store(&cx, &directory).unwrap();
    let card = store.card("cookbook/load/numbers/cas").unwrap();
    assert_eq!(card.book, "cookbook/loadable");
    assert_eq!(card.title, "Load numbers/cas");
    assert!(card.tags.contains(&"cookbook-action:load".to_owned()));
}

#[cfg(feature = "seed-recipes")]
#[test]
fn seeded_loadable_directory_loads_idempotently_and_unloads() {
    let directory = SeededLibCatalog::loadable_libs();
    let mut cx = setup_cx();
    let store = projected_recipe_store(&cx, &directory).unwrap();
    let load = store.card("cookbook/load/numbers/i64").unwrap();

    let first = run_recipe_with_loadable_libs(&mut cx, &directory, load).unwrap();
    let second = run_recipe_with_loadable_libs(&mut cx, &directory, load).unwrap();
    let matching = cx
        .registry()
        .libs()
        .iter()
        .filter(|loaded| loaded.manifest.id == Symbol::qualified("numbers", "i64"))
        .count();

    assert!(first.ok, "first load run: {first:?}");
    assert_eq!(first.results, ["loaded numbers/i64"]);
    assert!(second.ok, "second load run: {second:?}");
    assert_eq!(second.results, ["already loaded numbers/i64"]);
    assert_eq!(matching, 1);
    assert!(LoadableLibList::is_loaded(&cx, "numbers/i64"));

    let loaded_store = projected_recipe_store(&cx, &directory).unwrap();
    assert!(loaded_store.card("cookbook/load/numbers/i64").is_none());
    assert!(
        loaded_store
            .card("numbers/i64/01-basics/i64-domain")
            .is_some()
    );
    let unload = loaded_store
        .card("numbers/i64/cookbook-lifecycle/unload")
        .unwrap();
    let unload_run = run_recipe_with_loadable_libs(&mut cx, &directory, unload).unwrap();
    assert!(unload_run.ok, "unload run: {unload_run:?}");
    assert_eq!(unload_run.results, ["unloaded numbers/i64"]);

    let restored_store = projected_recipe_store(&cx, &directory).unwrap();
    assert!(restored_store.card("cookbook/load/numbers/i64").is_some());
    assert!(
        restored_store
            .card("numbers/i64/cookbook-lifecycle/unload")
            .is_none()
    );
    assert!(!LoadableLibList::is_loaded(&cx, "numbers/i64"));
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
    assert_eq!(run.checks.len(), card.expect.len());
    assert!(run.checks.iter().all(|check| check.pass));
    for expectation in &card.expect {
        assert_eq!(run.results.get(expectation.form), Some(&expectation.result));
    }
}

// ---- Requires-driven loading and capability profile ----

use crate::catalog::{CookbookCapabilityProfile, LibCatalog};
use crate::run::{run_recipe_twice, run_recipe_with_catalog};
#[cfg(feature = "seed-recipes")]
use crate::seed_catalog::SeededLibCatalog;

// A recipe that needs a domain absent from the eval Cx: `math/add` over i64,
// requiring `numbers/arith` + `numbers/i64`. `setup_cx` loads neither, so the
// recipe can only compute if the catalog LOADS them per its `requires`.
fn add_book() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("book.toml", b"book = \"t\"\ntitle = \"T\"\n" as &[u8]),
        (
            "c/add/recipe.toml",
            b"id = \"add\"\ntitle = \"Add\"\ncodec = \"lisp\"\nsetup = \"s\"\npurpose = \"p\"\nrequires = [\"numbers/arith\", \"numbers/i64\"]\n[[expect]]\nform = 0\nresult = \"3\"\n",
        ),
        ("c/add/s", b"(math/add 1 2)"),
        ("c/add/p", b"add two i64 literals"),
    ]
}

// Category A: the catalog LOADS a recipe's `requires` before eval, so a pure
// compute whose domain is absent from the base Cx runs to a real value.
#[cfg(feature = "seed-recipes")]
#[test]
fn requires_driven_loading_computes() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    let card = store_with(&add_book()).card("t/c/add").cloned().unwrap();
    let catalog = SeededLibCatalog::standard();
    let run = run_recipe_with_catalog(&mut cx, &catalog, &card).unwrap();
    assert!(run.ok, "expected add via catalog to pass, got {run:?}");
    assert_eq!(run.results, ["3"]);
    assert!(run.checks[0].pass);
}

// Without a catalog, the same recipe cannot load its domain, so it stays a
// descriptor: the runner reports the unresolved require rather than pretending
// to run.
#[test]
fn unresolved_require_is_descriptor() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    let card = store_with(&add_book()).card("t/c/add").cloned().unwrap();
    let err = run_recipe(&mut cx, &card).unwrap_err();
    match err {
        Error::Eval(message) => assert!(
            message.contains("descriptor: requires not in catalog")
                && message.contains("numbers/arith"),
            "{message}"
        ),
        other => panic!("expected descriptor Eval error, got {other:?}"),
    }
}

// Category C guard: running a deterministic recipe twice under the same
// (catalog + Cx) yields identical results, so the twice-run harness passes. A
// non-deterministic recipe would surface as an Eval error here.
#[cfg(feature = "seed-recipes")]
#[test]
fn twice_run_determinism_holds() {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    let card = store_with(&add_book()).card("t/c/add").cloned().unwrap();
    let catalog = SeededLibCatalog::standard();
    let run = run_recipe_twice(&mut cx, &catalog, &card).unwrap();
    assert!(run.ok, "twice-run determinism: {run:?}");
    assert_eq!(run.results, ["3"]);
}

// The capability profile grants the pure/offline vocabulary and denies the
// live/effectful one; seating a Cx through a host GrantSeat installs exactly the
// granted set, so a denied capability (Category D) is absent and fails closed.
#[test]
fn capability_profile_grants_and_denies() {
    use sim_kernel::{CapabilityName, GrantSeat};

    let read_construct = sim_kernel::read_construct_capability();
    let net_http = CapabilityName::new("net/http");
    let net_connect = CapabilityName::new("net-connect");
    assert!(CookbookCapabilityProfile::grants(&read_construct));
    assert!(CookbookCapabilityProfile::grants(&read_eval_capability()));
    assert!(CookbookCapabilityProfile::denies(&net_http));
    assert!(CookbookCapabilityProfile::denies(&net_connect));
    assert!(!CookbookCapabilityProfile::grants(&net_connect));

    let mut cx = setup_cx();
    let seat = GrantSeat::for_test();
    CookbookCapabilityProfile::seat(&seat, &mut cx).unwrap();
    assert!(cx.capabilities().contains(&read_construct));
    assert!(cx.capabilities().contains(&read_eval_capability()));
    // Category D: a denied capability is never seated, so an op demanding it
    // fails closed.
    assert!(!cx.capabilities().contains(&net_connect));
}

// Eval-policy organs (let/if/seq/match) run in the cookbook sandbox once their
// lib loads via `requires`. Each organ recipe is exercised through the catalog
// exactly as the webui serve path runs it.
#[cfg(feature = "seed-recipes")]
use sim_cookbook::RecipeRun;

#[cfg(feature = "seed-recipes")]
fn organ_book(id: &str, setup: &str, requires: &str, expect: &str) -> Vec<(String, Vec<u8>)> {
    codec_book(id, "lisp", setup, requires, expect)
}

#[cfg(feature = "seed-recipes")]
fn codec_book(
    id: &str,
    codec: &str,
    setup: &str,
    requires: &str,
    expect: &str,
) -> Vec<(String, Vec<u8>)> {
    let recipe = format!(
        "id = \"{id}\"\ntitle = \"{id}\"\ncodec = \"{codec}\"\nsetup = \"s\"\npurpose = \"p\"\nrequires = [{requires}]\n[[expect]]\nform = 0\nresult = \"{expect}\"\n",
    );
    vec![
        (
            "book.toml".to_owned(),
            b"book = \"organ\"\ntitle = \"Organ\"\n".to_vec(),
        ),
        (format!("c/{id}/recipe.toml"), recipe.into_bytes()),
        (format!("c/{id}/s"), setup.as_bytes().to_vec()),
        (format!("c/{id}/p"), b"organ".to_vec()),
    ]
}

#[cfg(feature = "seed-recipes")]
fn run_organ(setup: &str, requires: &str, expect: &str) -> RecipeRun {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    // The webui bootloader grants read-construct on the cookbook Cx (cli.rs), so
    // `#(numbers/Func ...)` reader constructs build a real value; mirror that here.
    cx.grant(read_construct_capability());
    let book = organ_book("r", setup, requires, expect);
    let refs: Vec<(&str, &[u8])> = book
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let card = store_with(&refs).card("organ/c/r").cloned().unwrap();
    let catalog = SeededLibCatalog::standard();
    run_recipe_twice(&mut cx, &catalog, &card).unwrap()
}

#[cfg(feature = "seed-recipes")]
fn run_codec(codec: &str, setup: &str, requires: &str, expect: &str) -> RecipeRun {
    let mut cx = setup_cx();
    cx.grant(read_eval_capability());
    cx.grant(read_construct_capability());
    let book = codec_book("r", codec, setup, requires, expect);
    let refs: Vec<(&str, &[u8])> = book
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let card = store_with(&refs).card("organ/c/r").cloned().unwrap();
    let catalog = SeededLibCatalog::standard();
    run_recipe_twice(&mut cx, &catalog, &card).unwrap()
}

// A recipe whose `codec` is a language surface parses+evals on that surface once
// the codec loads via `requires`. The conformance surfaces become "this surface
// computes X".
#[cfg(feature = "seed-recipes")]
#[test]
fn codec_algol_computes() {
    let run = run_codec(
        "algol",
        "1 + 2 * 3",
        "\"codec/algol\", \"numbers/arith\", \"numbers/i64\"",
        "7",
    );
    assert!(run.ok, "algol: {run:?}");
    assert_eq!(run.results, ["7"]);
}

// An offline MIDI render reduced to a deterministic frame digest runs green and
// reproduces under the twice-run guard.
#[cfg(feature = "seed-recipes")]
#[test]
fn category_c_midi_digest_computes() {
    let run = run_organ(
        "(midi/chord-digest \"60\")",
        "\"midi/digest\"",
        "\\\"(frame (bytes 9) (hash 605946920012b4cc))\\\"",
    );
    assert!(run.ok, "midi digest: {run:?}");
    assert_eq!(
        run.results,
        ["\"(frame (bytes 9) (hash 605946920012b4cc))\""]
    );
}

#[cfg(feature = "seed-recipes")]
#[test]
fn codec_scheme_computes() {
    let run = run_codec(
        "scheme-r7rs-small",
        "(+ 1 2)",
        "\"codec/scheme-r7rs-small\", \"numbers/arith\", \"numbers/i64\"",
        "3",
    );
    assert!(run.ok, "scheme: {run:?}");
    assert_eq!(run.results, ["3"]);
}

#[cfg(feature = "seed-recipes")]
#[test]
fn organ_let_computes() {
    let run = run_organ(
        "(let ((x 5)) (math/mul x x))",
        "\"binding\", \"numbers/arith\", \"numbers/i64\"",
        "25",
    );
    assert!(run.ok, "let: {run:?}");
    assert_eq!(run.results, ["25"]);
}

#[cfg(feature = "seed-recipes")]
#[test]
fn organ_if_computes() {
    let run = run_organ("(if true 10 20)", "\"control\"", "10");
    assert!(run.ok, "if: {run:?}");
    assert_eq!(run.results, ["10"]);
}

#[cfg(feature = "seed-recipes")]
#[test]
fn organ_seq_map_computes() {
    let run = run_organ(
        "(seq/map #(numbers/Func (x) (* x x)) [1 2 3])",
        "\"sequence\", \"numbers/func\", \"numbers/arith\", \"numbers/i64\"",
        "(1 4 9)",
    );
    assert!(run.ok, "seq/map: {run:?}");
    assert_eq!(run.results, ["(1 4 9)"]);
}

#[cfg(feature = "seed-recipes")]
#[test]
fn organ_match_computes() {
    let run = run_organ("(match [1 2] ([a b] a))", "\"pattern\"", "1");
    assert!(run.ok, "match: {run:?}");
    assert_eq!(run.results, ["1"]);
}
