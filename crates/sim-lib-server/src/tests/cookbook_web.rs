use sim_codec_lisp::LispCodecLib;
use sim_kernel::{Cx, read_eval_capability};
use sim_test_support::core_cx;

use crate::CookbookWebState;

fn lisp_cx() -> Cx {
    let mut cx = core_cx();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    cx.grant(read_eval_capability());
    cx
}

#[test]
fn cookbook_dynamic_api_lists_load_recipe_for_unloaded_lib() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request("GET", "/api/cookbook", Some(&mut cx));

    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
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
    assert!(api.body.contains("\"books\":[]"), "{}", api.body);
    assert!(api.body.contains("\"recipes\":[]"), "{}", api.body);
}
