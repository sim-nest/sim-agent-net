use std::sync::Arc;

use sim_cookbook::{EmbeddedDir, RecipeStore, ordered_cards};
use sim_kernel::{
    AbiVersion, Args, Callable, ClassRef, Cx, Dependency, Export, LibManifest, LibTarget, Linker,
    LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version, library::Lib,
    read_eval_capability,
};
use sim_test_support::core_cx;

use crate::{
    LibCatalog, LifecycleAction, LoadableLibEntry, LoadableLibList, lifecycle_action,
    projected_recipe_store, run_recipe_with_loadable_libs,
};

static DEMO_RECIPES: EmbeddedDir = &[
    ("book.toml", b"book = \"demo/lib\"\ntitle = \"Demo Lib\"\n"),
    (
        "01-basics/demo/recipe.toml",
        b"id = \"demo\"\ntitle = \"Demo\"\ncodec = \"lisp\"\nsetup = \"setup.siml\"\npurpose = \"purpose.md\"\n",
    ),
    ("01-basics/demo/setup.siml", b"(quote demo)"),
    ("01-basics/demo/purpose.md", b"demo recipe"),
];

struct FixtureLib {
    namespace: &'static str,
    name: &'static str,
    requires: Vec<Dependency>,
    export_probe: bool,
}

struct ProbeCallable;

impl Object for ProbeCallable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<probe-callable>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for ProbeCallable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.resolve_class(&Symbol::qualified("core", "Function"))
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for ProbeCallable {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        cx.factory().nil()
    }
}

impl Lib for FixtureLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified(self.namespace, self.name),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: self.requires.clone(),
            capabilities: Vec::new(),
            exports: if self.export_probe {
                vec![Export::Function {
                    symbol: probe_symbol(self.namespace, self.name),
                    function_id: None,
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker) -> Result<()> {
        if self.export_probe {
            linker.function_value(
                probe_symbol(self.namespace, self.name),
                cx.factory().opaque(Arc::new(ProbeCallable))?,
            )?;
        }
        Ok(())
    }
}

fn fixture_lib() -> Box<dyn Lib + Send + Sync> {
    Box::new(FixtureLib {
        namespace: "demo",
        name: "lib",
        requires: Vec::new(),
        export_probe: true,
    })
}

fn dependent_lib() -> Box<dyn Lib + Send + Sync> {
    Box::new(FixtureLib {
        namespace: "demo",
        name: "consumer",
        requires: vec![Dependency {
            id: Symbol::qualified("demo", "lib"),
            minimum_version: None,
        }],
        export_probe: false,
    })
}

fn probe_symbol(namespace: &str, name: &str) -> Symbol {
    Symbol::qualified(namespace.to_owned(), format!("{name}-probe"))
}

fn directory() -> LoadableLibList {
    LoadableLibList::new(vec![LoadableLibEntry {
        id: "demo/lib".to_owned(),
        source: "symbol:demo/lib".to_owned(),
        title: "Demo Lib".to_owned(),
        order: 7,
        recipes: Some(DEMO_RECIPES),
        catalog_lib: fixture_lib(),
        factory: Arc::new(fixture_lib),
    }])
}

#[test]
fn resolves_requires_by_full_id_and_tail() {
    let directory = directory();

    assert!(directory.resolve("demo/lib").is_some());
    assert!(directory.resolve("lib").is_some());
    assert!(directory.resolve("missing").is_none());
}

#[test]
fn loaded_check_accepts_entry_tail_alias() {
    let mut cx = core_cx();
    let alias = FixtureLib {
        namespace: "sim",
        name: "binding",
        requires: Vec::new(),
        export_probe: false,
    };
    cx.load_lib(&alias).unwrap();

    assert!(LoadableLibList::is_loaded(&cx, "organ/binding"));
    assert!(LoadableLibList::is_loaded(&cx, "binding"));
    assert!(LoadableLibList::is_loaded(&cx, "sim-binding"));
    assert!(!LoadableLibList::is_loaded(&cx, "organ/control"));
}

#[test]
fn projected_store_contains_load_card_for_unloaded_lib() {
    let cx = core_cx();
    let store = projected_recipe_store(&cx, &directory()).unwrap();

    let cards = store.cards();
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.id, "cookbook/load/demo/lib");
    assert_eq!(card.book, "cookbook/loadable");
    assert!(card.tags.contains(&"cookbook-action:load".to_owned()));
    assert_eq!(card.book_order, 7);
}

#[test]
fn lifecycle_action_reads_load_and_unload_tags() {
    let mut cx = core_cx();
    let load_store = projected_recipe_store(&cx, &directory()).unwrap();
    let load = load_store.card("cookbook/load/demo/lib").unwrap();

    assert_eq!(
        lifecycle_action(load),
        Some((LifecycleAction::Load, "demo/lib".to_owned()))
    );

    let lib = fixture_lib();
    cx.load_lib(lib.as_ref()).unwrap();
    let unload_store = projected_recipe_store(&cx, &directory()).unwrap();
    let unload = unload_store
        .card("demo/lib/cookbook-lifecycle/unload")
        .unwrap();

    assert_eq!(
        lifecycle_action(unload),
        Some((LifecycleAction::Unload, "demo/lib".to_owned()))
    );
}

#[test]
fn projected_store_contains_loaded_recipes_and_unload_card() {
    let mut cx = core_cx();
    let lib = fixture_lib();
    cx.load_lib(lib.as_ref()).unwrap();

    let store = projected_recipe_store(&cx, &directory()).unwrap();

    assert!(store.card("demo/lib/01-basics/demo").is_some());
    let unload = store.card("demo/lib/cookbook-lifecycle/unload").unwrap();
    assert_eq!(unload.order, i64::MAX);
    assert_eq!(unload.chapter_order, i64::MAX);
    assert!(unload.tags.contains(&"cookbook-action:unload".to_owned()));
    let ids: Vec<&str> = ordered_cards(&store)
        .into_iter()
        .map(|card| card.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "demo/lib/01-basics/demo",
            "demo/lib/cookbook-lifecycle/unload"
        ]
    );
}

#[test]
fn load_recipe_loads_known_lib() {
    let mut cx = core_cx();
    let directory = directory();
    let store = projected_recipe_store(&cx, &directory).unwrap();
    let card = store.card("cookbook/load/demo/lib").unwrap();

    let run = run_recipe_with_loadable_libs(&mut cx, &directory, card).unwrap();

    assert!(run.ok, "load run: {run:?}");
    assert_eq!(run.results, ["loaded demo/lib"]);
    assert!(LoadableLibList::is_loaded(&cx, "demo/lib"));
    assert!(
        cx.registry()
            .function_by_symbol(&probe_symbol("demo", "lib"))
            .is_some()
    );
}

#[test]
fn lifecycle_op_wrapper_uses_same_load_path() {
    let mut cx = core_cx();
    let op = crate::ops::CookbookLifecycleOp::new(Arc::new(directory()), LifecycleAction::Load);
    let lib = cx.factory().string("demo/lib".to_owned()).unwrap();

    op.call(&mut cx, Args::new(vec![lib])).unwrap();

    assert!(LoadableLibList::is_loaded(&cx, "demo/lib"));
    assert!(
        cx.registry()
            .function_by_symbol(&probe_symbol("demo", "lib"))
            .is_some()
    );
}

#[test]
fn load_recipe_is_idempotent() {
    let mut cx = core_cx();
    let directory = directory();
    let store = projected_recipe_store(&cx, &directory).unwrap();
    let card = store.card("cookbook/load/demo/lib").unwrap();

    run_recipe_with_loadable_libs(&mut cx, &directory, card).unwrap();
    let second = run_recipe_with_loadable_libs(&mut cx, &directory, card).unwrap();
    let matching = cx
        .registry()
        .libs()
        .iter()
        .filter(|loaded| loaded.manifest.id == Symbol::qualified("demo", "lib"))
        .count();

    assert!(second.ok, "second load run: {second:?}");
    assert_eq!(second.results, ["already loaded demo/lib"]);
    assert_eq!(matching, 1);
}

#[test]
fn unload_recipe_retracts_loaded_lib_exports() {
    let mut cx = core_cx();
    let directory = directory();
    let load_store = projected_recipe_store(&cx, &directory).unwrap();
    let load = load_store.card("cookbook/load/demo/lib").unwrap();
    run_recipe_with_loadable_libs(&mut cx, &directory, load).unwrap();
    let unload_store = projected_recipe_store(&cx, &directory).unwrap();
    let unload = unload_store
        .card("demo/lib/cookbook-lifecycle/unload")
        .unwrap();

    let run = run_recipe_with_loadable_libs(&mut cx, &directory, unload).unwrap();

    assert!(run.ok, "unload run: {run:?}");
    assert_eq!(run.results, ["unloaded demo/lib"]);
    assert!(!LoadableLibList::is_loaded(&cx, "demo/lib"));
    assert!(
        cx.registry()
            .function_by_symbol(&probe_symbol("demo", "lib"))
            .is_none()
    );
}

#[test]
fn unload_recipe_refuses_dependents() {
    let mut cx = core_cx();
    let directory = directory();
    let base = fixture_lib();
    let dependent = dependent_lib();
    cx.load_lib(base.as_ref()).unwrap();
    cx.load_lib(dependent.as_ref()).unwrap();
    let store = projected_recipe_store(&cx, &directory).unwrap();
    let unload = store.card("demo/lib/cookbook-lifecycle/unload").unwrap();

    let run = run_recipe_with_loadable_libs(&mut cx, &directory, unload).unwrap();

    assert!(!run.ok, "dependent unload should fail: {run:?}");
    assert!(run.results[0].contains("cannot unload demo/lib"));
    assert!(run.results[0].contains("consumer"));
    assert!(LoadableLibList::is_loaded(&cx, "demo/lib"));
    assert!(LoadableLibList::is_loaded(&cx, "demo/consumer"));
}

#[test]
fn ordinary_recipe_still_runs_through_catalog() {
    let mut cx = core_cx();
    let lisp = sim_codec_lisp::LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    cx.grant(read_eval_capability());
    let mut store = RecipeStore::new();
    store.register_book(DEMO_RECIPES).unwrap();
    let card = store.card("demo/lib/01-basics/demo").unwrap();
    let directory = directory();

    let run = run_recipe_with_loadable_libs(&mut cx, &directory, card).unwrap();

    assert!(run.ok, "ordinary recipe run: {run:?}");
    assert_eq!(run.results, ["demo"]);
    assert!(LoadableLibList::is_loaded(&cx, "demo/lib"));
}
