use super::support::{eval_cx, install_agent_lib, install_test_codec};
use sim_kernel::{Args, Expr, Symbol, Value};
use sim_lib_agent_runner_core::ModelResponse;
use sim_shape::{AnyShape, shape_value};
use std::sync::Arc;

fn model_request_expr(task: &str) -> Expr {
    Expr::Map(vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String(task.to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
    ])
}

fn echo_runner(cx: &mut sim_kernel::Cx, name: &str, model: &str) -> Value {
    cx.call_function(
        &Symbol::qualified("runner", "echo"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new(name)).unwrap(),
            cx.factory().symbol(Symbol::new(":model")).unwrap(),
            cx.factory().string(model.to_owned()).unwrap(),
        ]),
    )
    .unwrap()
}

fn expr_value(cx: &mut sim_kernel::Cx, expr: Expr) -> Value {
    cx.factory().expr(expr).unwrap()
}

fn field<'a>(expr: &'a Expr, name: &str) -> &'a Expr {
    let Expr::Map(entries) = expr else {
        panic!("expected map, found {expr:?}");
    };
    entries
        .iter()
        .find_map(|(key, value)| {
            matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == name).then_some(value)
        })
        .unwrap_or_else(|| panic!("missing field {name} in {expr:?}"))
}

#[test]
fn manifest_selected_conduct_routes_calls_and_reflects_contract() {
    let mut cx = eval_cx();
    cx.grant_named("agent-spawn");
    cx.grant_named("agent-reflect");
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let conduct = echo_runner(&mut cx, "conduct-seat", "conduct/model");
    let fallback = echo_runner(&mut cx, "Provider_4", "fallback/model");
    let result_shape_symbol = Symbol::qualified("shape", "text");
    cx.registry_mut()
        .register_shape_value(
            result_shape_symbol.clone(),
            shape_value(result_shape_symbol.clone(), Arc::new(AnyShape)),
        )
        .unwrap();
    let result_shape = cx.factory().symbol(result_shape_symbol.clone()).unwrap();
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("conduct-agent")).unwrap(),
                cx.factory().symbol(Symbol::new(":conduct")).unwrap(),
                conduct,
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                fallback,
                cx.factory().symbol(Symbol::new(":budget")).unwrap(),
                cx.factory().string("3".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":result-shape")).unwrap(),
                result_shape,
            ]),
        )
        .unwrap();

    let request = expr_value(&mut cx, model_request_expr("use the conduct path"));
    let reply = cx
        .call_function(
            &Symbol::qualified("agent", "call"),
            Args::new(vec![agent.clone(), request]),
        )
        .unwrap();
    let response = ModelResponse::try_from(reply.object().as_expr(&mut cx).unwrap()).unwrap();
    assert_eq!(response.runner, Symbol::new("conduct-seat"));
    assert_eq!(response.model, "conduct/model");

    let reflected = cx
        .call_function(
            &Symbol::qualified("agent", "reflect"),
            Args::new(vec![agent]),
        )
        .unwrap();
    let reflected = reflected.object().as_expr(&mut cx).unwrap();
    assert_eq!(
        field(&reflected, "conduct-id"),
        &Expr::Symbol(Symbol::new("conduct-seat"))
    );
    assert!(matches!(
        field(&reflected, "graph-fingerprint"),
        Expr::String(value) if value.starts_with("agent-graph:")
    ));
    assert_eq!(
        field(&reflected, "budget-defaults"),
        &Expr::String("3".to_owned())
    );
    assert_eq!(
        field(&reflected, "result-contract"),
        &Expr::Symbol(result_shape_symbol)
    );
    assert_eq!(field(&reflected, "required-roles"), &Expr::List(Vec::new()));
    assert!(matches!(
        field(&reflected, "effective-capabilities"),
        Expr::List(_)
    ));
}

#[test]
fn manifest_conduct_rejects_unresolved_required_roles_before_execution() {
    let mut cx = eval_cx();
    cx.grant_named("agent-spawn");
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let conduct = expr_value(
        &mut cx,
        Expr::Map(vec![(
            Expr::Symbol(Symbol::new("required-roles")),
            Expr::List(vec![Expr::Symbol(Symbol::new("planner"))]),
        )]),
    );
    let fallback = echo_runner(&mut cx, "Provider_4", "fallback/model");
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory()
                    .symbol(Symbol::new("invalid-conduct-agent"))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":conduct")).unwrap(),
                conduct,
                cx.factory().symbol(Symbol::new(":runners")).unwrap(),
                fallback,
            ]),
        )
        .unwrap();

    let request = expr_value(&mut cx, Expr::String("blocked".into()));
    let error = cx
        .call_function(
            &Symbol::qualified("agent", "call"),
            Args::new(vec![agent, request]),
        )
        .unwrap_err();
    assert!(
        format!("{error}").contains("agent conduct requires unresolved role planner"),
        "{error}"
    );
}
