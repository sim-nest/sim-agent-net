use std::sync::Arc;

use serde_json::json;
use sim_kernel::{Args, Cx, DefaultFactory, EagerPolicy, Error, Expr, Result, Symbol, Value};

use super::FixtureHttpTransport;
use crate::{
    SkillTransport, install_skill_lib, skill_call_capability, skill_call_symbol,
    skill_install_capability, skill_install_symbol, skill_specific_call_capability,
};

#[test]
fn openapi_fixture_discovery_creates_stable_skill_cards() {
    let mut cx = skill_cx();
    let transport = http_transport();

    let cards = transport.discover(&mut cx).unwrap();

    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.id, "petstore.findPets");
    assert_eq!(card.symbol, Symbol::qualified("skill", "petstore.findPets"));
    assert_eq!(card.transport_kind, "http-fixture");
    assert_eq!(card.operation, "findPets");
    assert!(card.description.contains("post /pets/search"));
}

#[test]
fn http_fixture_skill_call_goes_through_skill_callable() {
    let mut cx = skill_cx();
    grant_skill_caps(&mut cx, "petstore.findPets");
    let transport = install_http_skill(&mut cx);
    let target = cx.factory().string("petstore.findPets".to_owned()).unwrap();
    let request = request_value(&mut cx, "milo", 2);

    let result = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, request]))
        .unwrap();
    let expr = result.object().as_expr(&mut cx).unwrap();

    assert_eq!(map_string(&expr, "status"), Some("ok".to_owned()));
    assert_eq!(map_number(&expr, "count"), Some("1".to_owned()));
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn request_shape_failure_does_not_call_http_fixture() {
    let mut cx = skill_cx();
    grant_skill_caps(&mut cx, "petstore.findPets");
    let transport = install_http_skill(&mut cx);
    let target = cx.factory().string("petstore.findPets".to_owned()).unwrap();
    let limit = number_value(&mut cx, 2);
    let bad_request = cx
        .factory()
        .table(vec![(Symbol::new("limit"), limit)])
        .unwrap();

    let err = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, bad_request]))
        .unwrap_err();

    assert_wrong_shape(err);
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn raw_payload_is_not_exposed_without_raw_log_capability() {
    let mut cx = skill_cx();
    grant_skill_caps(&mut cx, "petstore.findPets");
    let transport = install_http_skill(&mut cx);
    let target = cx.factory().string("petstore.findPets".to_owned()).unwrap();
    let secret = "secret-token";
    let request = request_value(&mut cx, secret, 1);

    let result = cx
        .call_function(&skill_call_symbol(), Args::new(vec![target, request]))
        .unwrap();
    let health = transport.health(&mut cx).unwrap();

    assert!(!format!("{:?}", result.object().as_expr(&mut cx).unwrap()).contains(secret));
    assert!(!format!("{:?}", health.object().as_expr(&mut cx).unwrap()).contains(secret));
}

#[test]
#[ignore = "set SIM_SKILL_HTTP_LIVE_OPENAPI_JSON to an OpenAPI JSON file"]
fn live_openapi_discovery_smoke_is_opt_in() {
    let Some(path) = std::env::var_os("SIM_SKILL_HTTP_LIVE_OPENAPI_JSON") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let document = serde_json::from_slice(&bytes).unwrap();
    let transport = FixtureHttpTransport::from_openapi_value("live", document).unwrap();
    let mut cx = skill_cx();

    assert!(!transport.discover(&mut cx).unwrap().is_empty());
}

fn skill_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    install_skill_lib(&mut cx).unwrap();
    cx
}

fn grant_skill_caps(cx: &mut Cx, id: &str) {
    cx.grant(skill_install_capability());
    cx.grant(skill_call_capability());
    cx.grant(skill_specific_call_capability(id));
}

fn install_http_skill(cx: &mut Cx) -> Arc<FixtureHttpTransport> {
    cx.grant(skill_install_capability());
    let transport = Arc::new(http_transport());
    let transport_value = transport.clone().value(cx).unwrap();
    let cards = transport
        .discover(cx)
        .unwrap()
        .into_iter()
        .map(|card| card.value(cx))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let mut args = vec![transport_value];
    args.extend(cards);
    cx.call_function(&skill_install_symbol(), Args::new(args))
        .unwrap();
    transport
}

fn http_transport() -> FixtureHttpTransport {
    FixtureHttpTransport::from_openapi_value("petstore", openapi_doc())
        .unwrap()
        .with_response(
            "findPets",
            json!({
                "status": "ok",
                "count": 1,
                "items": [{"name": "Milo"}]
            }),
        )
}

fn openapi_doc() -> serde_json::Value {
    json!({
        "openapi": "3.1.0",
        "paths": {
            "/pets/search": {
                "post": {
                    "operationId": "findPets",
                    "summary": "Find pets",
                    "description": "Search pets without network access.",
                    "requestBody": {
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["query", "limit"],
                                    "properties": {
                                        "query": {"type": "string"},
                                        "limit": {"type": "integer"}
                                    }
                                }
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "required": ["status", "count", "items"],
                                        "properties": {
                                            "status": {"type": "string"},
                                            "count": {"type": "integer"},
                                            "items": {
                                                "type": "array",
                                                "items": {
                                                    "type": "object",
                                                    "required": ["name"],
                                                    "properties": {
                                                        "name": {"type": "string"}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

fn request_value(cx: &mut Cx, query: &str, limit: u32) -> Value {
    let query = cx.factory().string(query.to_owned()).unwrap();
    let limit = number_value(cx, limit);
    cx.factory()
        .table(vec![
            (Symbol::new("query"), query),
            (Symbol::new("limit"), limit),
        ])
        .unwrap()
}

fn number_value(cx: &mut Cx, value: u32) -> Value {
    cx.factory()
        .number_literal(Symbol::qualified("numbers", "f64"), value.to_string())
        .unwrap()
}

fn map_string(expr: &Expr, key: &str) -> Option<String> {
    map_field(expr, key).and_then(|value| match value {
        Expr::String(text) => Some(text.clone()),
        _ => None,
    })
}

fn map_number(expr: &Expr, key: &str) -> Option<String> {
    map_field(expr, key).and_then(|value| match value {
        Expr::Number(number) => Some(number.canonical.clone()),
        _ => None,
    })
}

use sim_value::access::field as map_field;

fn assert_wrong_shape(err: Error) {
    let Error::WrongShape { diagnostics, .. } = err else {
        panic!("expected WrongShape");
    };
    assert!(!diagnostics.is_empty());
}
