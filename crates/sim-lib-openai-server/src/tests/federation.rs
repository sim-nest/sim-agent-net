use std::sync::Arc;

use serde_json::Value;
use sim_kernel::{EvalFabricRef, Expr, Symbol};
use sim_lib_agent_runner_core::ModelResponse;

use crate::{
    GatewayEvent, OpenAiFederation, OpenAiGatewayFabric, eval_plan_report_with_federation,
    openai_gateway_federate_capability, openai_gateway_plan_capability, parse_plan,
};

#[test]
fn race_calls_two_in_process_gateway_fabrics_and_returns_first_answer() {
    let (federation, gateway_a, gateway_b) = federation_with_two_gateways();
    let mut cx = plan_federation_cx();
    let plan = parse_plan("race(gateway/a, gateway/b)").unwrap();
    let request = model_request("federate me", Vec::new());

    let report = eval_plan_report_with_federation(&mut cx, &plan, &request, &federation).unwrap();
    let response = ModelResponse::try_from(report.response).unwrap();

    assert_eq!(response.model, "gateway/a");
    assert!(format!("{:?}", response.content).contains("fixture a transcript"));
    assert!(gateway_a.last_execution().unwrap().is_some());
    assert!(gateway_b.last_execution().unwrap().is_some());
}

#[test]
fn federation_requires_federate_capability() {
    let (federation, _, _) = federation_with_two_gateways();
    let mut cx = super::cx();
    cx.grant(openai_gateway_plan_capability());
    let plan = parse_plan("race(gateway/a, gateway/b)").unwrap();
    let request = model_request("federate me", Vec::new());

    let err = eval_plan_report_with_federation(&mut cx, &plan, &request, &federation).unwrap_err();

    assert!(format!("{err}").contains("openai-gateway.federate"));
}

#[test]
fn local_only_request_refuses_federation_boundary() {
    let (federation, gateway_a, gateway_b) = federation_with_two_gateways();
    let mut cx = plan_federation_cx();
    let plan = parse_plan("race(gateway/a, gateway/b)").unwrap();
    let request = model_request(
        "stay local",
        vec![(
            Expr::Symbol(Symbol::new("privacy")),
            Expr::String("local-only".to_owned()),
        )],
    );

    let err = eval_plan_report_with_federation(&mut cx, &plan, &request, &federation).unwrap_err();

    assert!(format!("{err}").contains("local-only privacy"));
    assert!(gateway_a.last_execution().unwrap().is_none());
    assert!(gateway_b.last_execution().unwrap().is_none());
}

#[test]
fn federation_carries_privacy_and_budget_policy_to_target_request() {
    let (federation, gateway_a, _) = federation_with_two_gateways();
    let mut cx = plan_federation_cx();
    let plan = parse_plan("budget(gateway/a, max-tokens: 7)").unwrap();
    let request = model_request(
        "federate with policy",
        vec![(
            Expr::Symbol(Symbol::new("privacy")),
            Expr::String("metadata-only".to_owned()),
        )],
    );

    eval_plan_report_with_federation(&mut cx, &plan, &request, &federation).unwrap();
    let body = target_request_json(&gateway_a);

    assert_eq!(body["model"], "fixture/a");
    assert_eq!(body["input"], "federate with policy");
    assert_eq!(body["privacy"], "metadata-only");
    assert_eq!(body["budget"]["max-tokens"], 7);
}

fn federation_with_two_gateways() -> (
    OpenAiFederation,
    Arc<OpenAiGatewayFabric>,
    Arc<OpenAiGatewayFabric>,
) {
    let federation = OpenAiFederation::new();
    let gateway_a = Arc::new(OpenAiGatewayFabric::deterministic(101, 10_000, 10));
    let gateway_b = Arc::new(OpenAiGatewayFabric::deterministic(201, 20_000, 10));
    let target_a: EvalFabricRef = gateway_a.clone();
    let target_b: EvalFabricRef = gateway_b.clone();
    federation
        .insert_gateway("gateway/a", "fixture/a", target_a)
        .unwrap();
    federation
        .insert_gateway("gateway/b", "fixture/b", target_b)
        .unwrap();
    (federation, gateway_a, gateway_b)
}

fn plan_federation_cx() -> sim_kernel::Cx {
    let mut cx = super::cx();
    cx.grant(openai_gateway_plan_capability());
    cx.grant(openai_gateway_federate_capability());
    cx
}

fn model_request(text: &str, extra: Vec<(Expr, Expr)>) -> Expr {
    let mut entries = vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(text.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ];
    entries.extend(extra);
    Expr::Map(entries)
}

fn target_request_json(gateway: &OpenAiGatewayFabric) -> Value {
    let execution = gateway.last_execution().unwrap().unwrap();
    let event = execution
        .events()
        .iter()
        .find(|event| event.kind().name.as_ref() == "request-start")
        .unwrap();
    let body = request_body(event);
    serde_json::from_slice(body).unwrap()
}

fn request_body(event: &GatewayEvent) -> &[u8] {
    let Expr::Map(entries) = event.payload() else {
        panic!("request-start payload must be a map");
    };
    entries
        .iter()
        .find_map(|(key, value)| match (key, value) {
            (Expr::Symbol(key), Expr::Bytes(body))
                if key.namespace.is_none() && key.name.as_ref() == "body" =>
            {
                Some(body.as_slice())
            }
            _ => None,
        })
        .unwrap()
}
