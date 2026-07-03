use super::support::{as_component, eval_cx, install_agent_lib, install_test_codec};
use crate::expr_to_value;
use sim_kernel::{Args, Consistency, Cx, EvalFabric, EvalMode, EvalRequest, Expr, Symbol, Value};

#[test]
fn a5_phase9_string_shape_accepts_text_response() {
    let mut cx = phase9_cx();
    let runner = fake_runner(&mut cx, vec![Expr::String("valid text".to_owned())]);
    let reply = as_component(&runner)
        .realize(
            &mut cx,
            EvalRequest {
                expr: request_expr(false, false, output_string_shape()),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::LocalFirst,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();
    let expr = reply.value.object().as_expr(&mut cx).unwrap();
    assert!(format!("{expr:?}").contains("shape-ok"));
    assert!(format!("{expr:?}").contains("valid text"));
}

#[test]
fn a5_phase9_table_shape_rejects_invalid_text_output() {
    let mut cx = phase9_cx();
    let runner = fake_runner(&mut cx, vec![Expr::String("not a table".to_owned())]);
    let reply = as_component(&runner)
        .realize(
            &mut cx,
            EvalRequest {
                expr: request_expr(false, false, output_package_shape()),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::LocalFirst,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();
    let expr = reply.value.object().as_expr(&mut cx).unwrap();
    assert!(format!("{expr:?}").contains("shape-ok"));
    assert!(format!("{expr:?}").contains("shape-diagnostics"));
}

#[test]
fn a5_phase9_fake_runner_repairs_into_valid_shape() {
    let mut cx = phase9_cx();
    let runner = fake_runner(
        &mut cx,
        vec![
            Expr::String("still not a table".to_owned()),
            submit_shape_call("repair-1", valid_package_expr()),
        ],
    );
    let reply = as_component(&runner)
        .realize(
            &mut cx,
            EvalRequest {
                expr: request_expr(true, true, output_package_shape()),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::LocalFirst,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();
    let expr = reply.value.object().as_expr(&mut cx).unwrap();
    assert!(format!("{expr:?}").contains("shape-repaired"));
    assert!(format!("{expr:?}").contains("submitted-value"));
}

#[test]
fn a5_phase9_submit_shape_rejects_invalid_value() {
    let mut cx = phase9_cx();
    let runner = fake_runner(
        &mut cx,
        vec![submit_shape_call(
            "reject-1",
            Expr::String("wrong".to_owned()),
        )],
    );
    let reply = as_component(&runner)
        .realize(
            &mut cx,
            EvalRequest {
                expr: request_expr(false, true, output_package_shape()),
                result_shape: None,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: Consistency::LocalFirst,
                mode: EvalMode::Eval,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                trace: false,
            },
        )
        .unwrap();
    let expr = reply.value.object().as_expr(&mut cx).unwrap();
    assert!(format!("{expr:?}").contains("error"));
    assert!(!format!("{expr:?}").contains("submitted-value"));
}

fn phase9_cx() -> Cx {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn fake_runner(cx: &mut Cx, script: Vec<Expr>) -> Value {
    let script_value = expr_to_value(cx, &Expr::List(script)).unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new("phase9-fake")).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn request_expr(repair: bool, submit_shape: bool, output_shape: Expr) -> Expr {
    let mut entries = vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (
            Expr::Symbol(Symbol::new("task")),
            Expr::String("shape-check".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(Vec::new()),
        ),
        (Expr::Symbol(Symbol::new("output-shape")), output_shape),
    ];
    if repair {
        entries.push((Expr::Symbol(Symbol::new("repair")), Expr::Bool(true)));
    }
    if submit_shape {
        entries.push((Expr::Symbol(Symbol::new("submit-shape")), Expr::Bool(true)));
    }
    Expr::Map(entries)
}

fn submit_shape_call(id: &str, submitted: Expr) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("model-response")),
            Expr::Bool(true),
        ),
        (
            Expr::Symbol(Symbol::new("runner")),
            Expr::Symbol(Symbol::new("phase9-fake")),
        ),
        (
            Expr::Symbol(Symbol::new("model")),
            Expr::String("runner/fake".to_owned()),
        ),
        (Expr::Symbol(Symbol::new("content")), Expr::List(Vec::new())),
        (
            Expr::Symbol(Symbol::new("stop-reason")),
            Expr::Symbol(Symbol::new("tool-call")),
        ),
        (
            Expr::Symbol(Symbol::new("tool-calls")),
            Expr::List(vec![Expr::Map(vec![
                (Expr::Symbol(Symbol::new("id")), Expr::String(id.to_owned())),
                (
                    Expr::Symbol(Symbol::new("name")),
                    Expr::Symbol(Symbol::new("submit_shape_value")),
                ),
                (
                    Expr::Symbol(Symbol::new("arguments")),
                    Expr::List(vec![submitted]),
                ),
            ])]),
        ),
    ])
}

fn output_string_shape() -> Expr {
    Expr::Symbol(Symbol::new("String"))
}

fn output_package_shape() -> Expr {
    Expr::List(vec![
        Expr::Symbol(Symbol::new("fields")),
        Expr::List(vec![
            Expr::Symbol(Symbol::new(":name")),
            Expr::Symbol(Symbol::new("String")),
        ]),
        Expr::List(vec![
            Expr::Symbol(Symbol::new(":version")),
            Expr::Symbol(Symbol::new("String")),
        ]),
    ])
}

fn valid_package_expr() -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("name")),
            Expr::String("sim-say".to_owned()),
        ),
        (
            Expr::Symbol(Symbol::new("version")),
            Expr::String("0.1.0".to_owned()),
        ),
    ])
}
