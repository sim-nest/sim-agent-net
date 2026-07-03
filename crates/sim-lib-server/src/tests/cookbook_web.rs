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
fn cookbook_api_index_returns_seeded_tree() {
    let state = CookbookWebState::seeded().unwrap();
    let response = state.handle_request("GET", "/api/cookbook", None);
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "application/json; charset=utf-8");
    assert!(response.body.contains("\"books\""), "{}", response.body);
    assert!(
        response.body.contains("codec/lisp/01-basics/quote-symbol"),
        "{}",
        response.body
    );
    assert!(response.body.contains("\"chapters\""), "{}", response.body);
}

#[test]
fn cookbook_api_search_filters_seeded_recipes() {
    let state = CookbookWebState::seeded().unwrap();
    let response = state.handle_request("GET", "/api/cookbook/search?q=symbol", None);
    assert_eq!(response.status, 200);
    assert!(
        response.body.contains("codec/lisp/01-basics/quote-symbol"),
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
    let response = state.handle_request(
        "GET",
        "/api/cookbook/recipe/codec%2Flisp%2F01-basics%2Fquote-symbol",
        None,
    );
    assert_eq!(response.status, 200);
    assert!(response.body.contains("\"setup\""), "{}", response.body);
    assert!(response.body.contains("codec-lisp-ok"), "{}", response.body);
    assert!(response.body.contains("\"next\""), "{}", response.body);
}

#[test]
fn cookbook_api_run_returns_pass_fail_data() {
    let state = CookbookWebState::seeded().unwrap();
    let mut cx = lisp_cx();
    let response = state.handle_request(
        "POST",
        "/api/cookbook/recipe/codec/lisp/01-basics/quote-symbol/run",
        Some(&mut cx),
    );
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(response.body.contains("\"ok\":true"), "{}", response.body);
    assert!(response.body.contains("\"checks\""), "{}", response.body);
    assert!(response.body.contains("codec-lisp-ok"), "{}", response.body);
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
    let page = state.handle_request("GET", "/cookbook", None);
    assert_eq!(page.status, 200);
    assert!(page.body.contains("No recipes loaded."), "{}", page.body);

    let api = state.handle_request("GET", "/api/cookbook", None);
    assert_eq!(api.status, 200);
    assert!(api.body.contains("\"books\":[]"), "{}", api.body);
    assert!(api.body.contains("\"recipes\":[]"), "{}", api.body);
}
