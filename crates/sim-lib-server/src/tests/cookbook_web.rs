use std::sync::Arc;

use sim_codec_lisp::LispCodecLib;
use sim_kernel::{
    AbiVersion, Cx, Dependency, Export, LibManifest, LibTarget, Linker, LoadCx, Result, Symbol,
    Version, library::Lib, read_eval_capability,
};
use sim_lib_cookbook::{LoadableLibEntry, LoadableLibList, SeededLibCatalog, built_in_config};
use sim_test_support::core_cx;

use crate::CookbookWebState;

fn lisp_cx() -> Cx {
    let mut cx = core_cx();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    cx.grant(read_eval_capability());
    cx
}

struct FixtureLib {
    name: &'static str,
    requires: Vec<Dependency>,
}

impl Lib for FixtureLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("demo", self.name),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: self.requires.clone(),
            capabilities: Vec::new(),
            exports: Vec::<Export>::new(),
        }
    }

    fn load(&self, _cx: &mut LoadCx, _linker: &mut Linker) -> Result<()> {
        Ok(())
    }
}

fn fixture_lib() -> Box<dyn Lib + Send + Sync> {
    Box::new(FixtureLib {
        name: "lib",
        requires: Vec::new(),
    })
}

fn dependent_lib() -> Box<dyn Lib + Send + Sync> {
    Box::new(FixtureLib {
        name: "consumer",
        requires: vec![Dependency {
            id: Symbol::qualified("demo", "lib"),
            minimum_version: None,
        }],
    })
}

fn fixture_directory() -> LoadableLibList {
    LoadableLibList::new(vec![LoadableLibEntry {
        id: "demo/lib".to_owned(),
        source: "symbol:demo/lib".to_owned(),
        title: "Demo Lib".to_owned(),
        order: 1,
        recipes: None,
        catalog_lib: fixture_lib(),
        factory: Arc::new(fixture_lib),
    }])
}

#[test]
fn cookbook_default_directory_and_startup_loaded_counts_are_current() {
    let directory = SeededLibCatalog::loadable_libs();
    let cx = lisp_cx();
    let loaded = directory
        .entries()
        .iter()
        .filter(|entry| LoadableLibList::is_loaded(&cx, &entry.id))
        .count();

    assert_eq!(built_in_config().minimum_loaded, ["codec/lisp"]);
    assert_eq!(directory.entries().len(), 19);
    assert_eq!(loaded, 0);
}

#[test]
fn cookbook_dynamic_api_lists_load_recipe_for_unloaded_lib() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request("GET", "/api/cookbook", Some(&mut cx));

    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    assert!(
        response.body.starts_with("{\"libs\":["),
        "{}",
        response.body
    );
    assert!(
        response.body.contains(
            "\"id\":\"numbers/i64\",\"title\":\"I64 numbers\",\"loaded\":false,\"recipes\":[{\"id\":\"cookbook/load/numbers/i64\""
        ),
        "{}",
        response.body
    );
    assert!(response.body.contains("\"books\""), "{}", response.body);
    assert!(
        response.body.contains("cookbook/load/numbers/i64"),
        "{}",
        response.body
    );
    assert!(response.body.contains("\"chapters\""), "{}", response.body);
    assert!(response.body.contains("\"families\""), "{}", response.body);
    assert!(
        response.body.contains("\"diagnostics\":[]"),
        "{}",
        response.body
    );
    assert!(
        response.body.contains("\"action\":\"load\""),
        "{}",
        response.body
    );
    assert!(
        response.body.contains("\"lib\":\"numbers/i64\""),
        "{}",
        response.body
    );
    assert!(
        response.body.contains("\"loaded\":false"),
        "{}",
        response.body
    );
}

#[test]
fn cookbook_api_index_keeps_grouped_and_flat_arrays() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request("GET", "/api/cookbook", Some(&mut cx));

    assert_eq!(response.status, 200);
    assert!(
        response.body.starts_with("{\"libs\":["),
        "{}",
        response.body
    );
    assert!(response.body.contains("\"families\""), "{}", response.body);
    assert!(
        response.body.contains("\"family\":\"cookbook\""),
        "{}",
        response.body
    );
    assert!(response.body.contains("\"recipes\":["), "{}", response.body);
    assert!(
        response.body.contains("\"runnable\":true"),
        "{}",
        response.body
    );
}

#[test]
fn cookbook_api_search_filters_seeded_recipes() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request("GET", "/api/cookbook/search?q=i64", Some(&mut cx));

    assert_eq!(response.status, 200);
    assert!(
        response.body.contains("cookbook/load/numbers/i64"),
        "{}",
        response.body
    );
    assert!(
        !response.body.contains("numbers/f64/01-basics/add-floats"),
        "{}",
        response.body
    );
}

#[test]
fn cookbook_api_recipe_detail_accepts_encoded_ids() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request(
        "GET",
        "/api/cookbook/recipe/cookbook%2Fload%2Fnumbers%2Fi64",
        Some(&mut cx),
    );

    assert_eq!(response.status, 200);
    assert!(response.body.contains("\"setup\""), "{}", response.body);
    assert!(response.body.contains("\"next\""), "{}", response.body);
    assert!(
        response.body.contains("\"action\":\"load\""),
        "{}",
        response.body
    );
    assert!(
        response.body.contains("\"lib\":\"numbers/i64\""),
        "{}",
        response.body
    );
    assert!(
        response.body.contains("\"loaded\":false"),
        "{}",
        response.body
    );
}

#[test]
fn cookbook_dynamic_api_load_changes_next_index_response() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request(
        "POST",
        "/api/cookbook/recipe/cookbook/load/numbers/i64/run",
        Some(&mut cx),
    );

    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.body.contains("\"ok\":true"), "{}", response.body);
    assert!(
        response.body.contains("loaded numbers/i64"),
        "{}",
        response.body
    );

    let index = state.handle_request("GET", "/api/cookbook", Some(&mut cx));
    assert_eq!(index.status, 200, "{}", index.body);
    assert!(
        index.body.contains(
            "\"id\":\"numbers/i64\",\"title\":\"I64 numbers\",\"loaded\":true,\"groups\":["
        ),
        "{}",
        index.body
    );
    assert!(
        index.body.contains("numbers/i64/01-basics/i64-domain"),
        "{}",
        index.body
    );
    assert!(
        index.body.contains("numbers/i64/cookbook-lifecycle/unload"),
        "{}",
        index.body
    );
    assert!(
        index.body.contains("\"action\":\"unload\""),
        "{}",
        index.body
    );
    assert!(
        index.body.contains("\"lib\":\"numbers/i64\""),
        "{}",
        index.body
    );
    assert!(index.body.contains("\"loaded\":true"), "{}", index.body);
    assert!(
        !index.body.contains("cookbook/load/numbers/i64"),
        "{}",
        index.body
    );
    let loaded_lib = index
        .body
        .find("\"id\":\"numbers/i64\",\"title\":\"I64 numbers\",\"loaded\":true")
        .unwrap();
    let loaded_recipe = index.body[loaded_lib..]
        .find("numbers/i64/01-basics/i64-domain")
        .unwrap();
    let unload_recipe = index.body[loaded_lib..]
        .find("numbers/i64/cookbook-lifecycle/unload")
        .unwrap();
    assert!(loaded_recipe < unload_recipe, "{}", index.body);
}

#[test]
fn cookbook_dynamic_api_load_is_idempotent() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let first = state.handle_request(
        "POST",
        "/api/cookbook/recipe/cookbook/load/numbers/i64/run",
        Some(&mut cx),
    );
    assert_eq!(first.status, 200, "{}", first.body);
    assert!(first.body.contains("\"ok\":true"), "{}", first.body);
    let first_index = state.handle_request("GET", "/api/cookbook", Some(&mut cx));
    assert_eq!(first_index.status, 200, "{}", first_index.body);
    let first_recipe_count = first_index
        .body
        .matches("\"id\":\"numbers/i64/01-basics/i64-domain\"")
        .count();
    let first_unload_count = first_index
        .body
        .matches("\"id\":\"numbers/i64/cookbook-lifecycle/unload\"")
        .count();

    let second = state.handle_request(
        "POST",
        "/api/cookbook/recipe/cookbook/load/numbers/i64/run",
        Some(&mut cx),
    );
    assert_eq!(second.status, 200, "{}", second.body);
    assert!(second.body.contains("\"ok\":true"), "{}", second.body);
    assert!(
        second.body.contains("already loaded numbers/i64"),
        "{}",
        second.body
    );

    let second_index = state.handle_request("GET", "/api/cookbook", Some(&mut cx));
    assert_eq!(second_index.status, 200, "{}", second_index.body);
    assert!(first_recipe_count > 0, "{}", first_index.body);
    assert!(first_unload_count > 0, "{}", first_index.body);
    assert_eq!(
        first_recipe_count,
        second_index
            .body
            .matches("\"id\":\"numbers/i64/01-basics/i64-domain\"")
            .count(),
        "{}",
        second_index.body
    );
    assert_eq!(
        first_unload_count,
        second_index
            .body
            .matches("\"id\":\"numbers/i64/cookbook-lifecycle/unload\"")
            .count(),
        "{}",
        second_index.body
    );
    assert!(
        !second_index.body.contains("cookbook/load/numbers/i64"),
        "{}",
        second_index.body
    );
}

#[test]
fn cookbook_loaded_lib_without_embedded_recipes_shows_setup_debt_before_unload() {
    let state = CookbookWebState::from_loadable_libs(fixture_directory(), Vec::new());
    let mut cx = core_cx();
    let load = state.handle_request(
        "POST",
        "/api/cookbook/recipe/cookbook/load/demo/lib/run",
        Some(&mut cx),
    );
    assert_eq!(load.status, 200, "{}", load.body);

    let index = state.handle_request("GET", "/api/cookbook", Some(&mut cx));
    assert_eq!(index.status, 200, "{}", index.body);
    assert!(
        index
            .body
            .contains("\"id\":\"demo/lib/cookbook-lifecycle/setup-debt\""),
        "{}",
        index.body
    );
    assert!(
        index
            .body
            .contains("\"id\":\"demo/lib/cookbook-lifecycle/unload\""),
        "{}",
        index.body
    );
    assert!(
        index
            .body
            .contains("\"id\":\"demo/lib/cookbook-lifecycle/setup-debt\",\"title\":\"Setup debt for demo/lib\",\"book\":\"demo/lib\",\"chapter\":\"cookbook-lifecycle\",\"runnable\":false"),
        "{}",
        index.body
    );
    let setup_debt = index
        .body
        .find("demo/lib/cookbook-lifecycle/setup-debt")
        .unwrap();
    let unload = index
        .body
        .find("demo/lib/cookbook-lifecycle/unload")
        .unwrap();
    assert!(setup_debt < unload, "{}", index.body);

    let detail = state.handle_request(
        "GET",
        "/api/cookbook/recipe/demo/lib/cookbook-lifecycle/setup-debt",
        Some(&mut cx),
    );
    assert_eq!(detail.status, 200, "{}", detail.body);
    assert!(
        detail.body.contains("setup-debt:missing-recipes"),
        "{}",
        detail.body
    );
    assert!(
        detail
            .body
            .contains("exposes no embedded cookbook directory"),
        "{}",
        detail.body
    );
}

#[test]
fn cookbook_dynamic_api_unload_changes_next_index_response() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let load = state.handle_request(
        "POST",
        "/api/cookbook/recipe/cookbook/load/numbers/i64/run",
        Some(&mut cx),
    );
    assert_eq!(load.status, 200, "{}", load.body);

    let unload = state.handle_request(
        "POST",
        "/api/cookbook/recipe/numbers/i64/cookbook-lifecycle/unload/run",
        Some(&mut cx),
    );

    assert_eq!(unload.status, 200, "{}", unload.body);
    assert!(unload.body.contains("\"ok\":true"), "{}", unload.body);
    assert!(
        unload.body.contains("unloaded numbers/i64"),
        "{}",
        unload.body
    );

    let index = state.handle_request("GET", "/api/cookbook", Some(&mut cx));
    assert_eq!(index.status, 200, "{}", index.body);
    assert!(
        index.body.contains("cookbook/load/numbers/i64"),
        "{}",
        index.body
    );
    assert!(
        !index.body.contains("numbers/i64/cookbook-lifecycle/unload"),
        "{}",
        index.body
    );
}

#[test]
fn cookbook_dynamic_api_safe_unload_refusal_is_json_run_result() {
    let state = CookbookWebState::from_loadable_libs(fixture_directory(), Vec::new());
    let mut cx = core_cx();
    let base = fixture_lib();
    let dependent = dependent_lib();
    cx.load_lib(base.as_ref()).unwrap();
    cx.load_lib(dependent.as_ref()).unwrap();

    let response = state.handle_request(
        "POST",
        "/api/cookbook/recipe/demo/lib/cookbook-lifecycle/unload/run",
        Some(&mut cx),
    );

    assert_eq!(response.status, 200, "{}", response.body);
    assert!(
        response
            .body
            .contains("\"recipe\":\"demo/lib/cookbook-lifecycle/unload\""),
        "{}",
        response.body
    );
    assert!(response.body.contains("\"ok\":false"), "{}", response.body);
    assert!(
        response.body.contains("cannot unload demo/lib"),
        "{}",
        response.body
    );
    assert!(response.body.contains("consumer"), "{}", response.body);
    assert!(LoadableLibList::is_loaded(&cx, "demo/lib"));
    assert!(LoadableLibList::is_loaded(&cx, "demo/consumer"));
}

#[test]
fn cookbook_dynamic_detail_and_search_use_projected_store() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let load = state.handle_request(
        "POST",
        "/api/cookbook/recipe/cookbook/load/numbers/i64/run",
        Some(&mut cx),
    );
    assert_eq!(load.status, 200, "{}", load.body);

    let detail = state.handle_request(
        "GET",
        "/api/cookbook/recipe/numbers%2Fi64%2F01-basics%2Fi64-domain",
        Some(&mut cx),
    );
    assert_eq!(detail.status, 200, "{}", detail.body);
    assert!(detail.body.contains("\"action\":null"), "{}", detail.body);
    assert!(
        detail.body.contains("\"lib\":\"numbers/i64\""),
        "{}",
        detail.body
    );
    assert!(detail.body.contains("\"loaded\":true"), "{}", detail.body);

    let search = state.handle_request("GET", "/api/cookbook/search?q=64-bit", Some(&mut cx));
    assert_eq!(search.status, 200, "{}", search.body);
    assert!(
        search.body.contains("numbers/i64/01-basics/i64-domain"),
        "{}",
        search.body
    );
}

#[test]
fn cookbook_api_run_route_rejects_get() {
    let state = CookbookWebState::seeded().unwrap();
    let response = state.handle_request(
        "GET",
        "/api/cookbook/recipe/codec/lisp/01-basics/quote-symbol/run",
        None,
    );
    assert_eq!(response.status, 405);
}

#[test]
fn cookbook_page_and_api_render_empty_state() {
    let state = CookbookWebState::empty();
    let mut cx = lisp_cx();
    let page = state.handle_request("GET", "/cookbook", Some(&mut cx));
    assert_eq!(page.status, 200);
    assert!(page.body.contains("No recipes loaded."), "{}", page.body);

    let api = state.handle_request("GET", "/api/cookbook", Some(&mut cx));
    assert_eq!(api.status, 200);
    assert!(api.body.contains("\"libs\":[]"), "{}", api.body);
    assert!(api.body.contains("\"books\":[]"), "{}", api.body);
    assert!(api.body.contains("\"recipes\":[]"), "{}", api.body);
}
