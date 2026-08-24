use super::support::{as_component, eval_cx, install_agent_lib, install_test_codec, request_frame};
use crate::tools::register_tool;
use crate::{Tool, value_from_expr};
use sim_kernel::{Args, Cx, Error, EventKind, Expr, Object, Result, Symbol, Value};
use sim_lib_server::{EvalSite, ServerAddress, eval_reply_from_frame};
use sim_shape::{AnyShape, ListShape, NumberValueShape, shape_value};
use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

// conformance: bounded agent tool turns continue once and stop at their declared budget.

#[test]
fn a5_phase8_fake_runner_continues_after_tool_call() {
    let mut cx = phase8_cx();
    cx.grant_named("math");
    register_strict_sum_tool(&mut cx);

    let runner = fake_runner(
        &mut cx,
        vec![
            tool_call_response(vec![tool_call(
                "call-1",
                Symbol::qualified("test", "sum"),
                vec![number(2), number(3)],
            )]),
            final_response("continued after tool"),
        ],
    );
    let frame = request_frame(
        &mut cx,
        request_expr(
            "sum",
            vec![tool_descriptor(Symbol::qualified("test", "sum"))],
            None,
        ),
    );
    let reply = as_component(&runner).answer(&mut cx, frame).unwrap();
    let (expr, diagnostics) = reply_expr_and_diagnostics(&mut cx, &reply);
    assert!(diagnostics.is_empty());
    assert!(format!("{expr:?}").contains("continued after tool"));
    let effects = cx.effect_ledger().events_for_run();
    let requested = effects
        .iter()
        .position(|event| matches!(event.kind, EventKind::EffectRequested { .. }))
        .expect("tool effect request recorded");
    let resolved = effects
        .iter()
        .position(|event| matches!(event.kind, EventKind::EffectResolved { .. }))
        .expect("tool effect resolution recorded");
    assert!(
        requested < resolved,
        "effect request must precede resolution"
    );
}

#[test]
fn a5_phase8_bad_tool_name_returns_model_error() {
    let mut cx = phase8_cx();
    let runner = fake_runner(
        &mut cx,
        vec![tool_call_response(vec![tool_call(
            "call-missing",
            Symbol::qualified("test", "missing"),
            Vec::new(),
        )])],
    );
    let frame = request_frame(
        &mut cx,
        request_expr(
            "missing",
            vec![tool_descriptor(Symbol::qualified("test", "missing"))],
            None,
        ),
    );
    let reply = as_component(&runner).answer(&mut cx, frame).unwrap();
    let (expr, _) = reply_expr_and_diagnostics(&mut cx, &reply);
    assert!(format!("{expr:?}").contains("unknown tool test/missing"));
    assert!(format!("{expr:?}").contains("stop-reason"));
    assert!(format!("{expr:?}").contains("error"));
}

#[test]
fn a5_phase8_wrong_tool_args_become_tool_result_error() {
    let mut cx = phase8_cx();
    cx.grant_named("math");
    register_strict_sum_tool(&mut cx);

    let runner = fake_runner(
        &mut cx,
        vec![
            tool_call_response(vec![tool_call(
                "call-bad-args",
                Symbol::qualified("test", "sum"),
                vec![Expr::String("not-a-number".to_owned())],
            )]),
            final_response("handled tool error"),
        ],
    );
    let frame = request_frame(
        &mut cx,
        request_expr(
            "bad args",
            vec![tool_descriptor(Symbol::qualified("test", "sum"))],
            None,
        ),
    );
    let reply = as_component(&runner).answer(&mut cx, frame).unwrap();
    let (expr, diagnostics) = reply_expr_and_diagnostics(&mut cx, &reply);
    assert!(format!("{expr:?}").contains("handled tool error"));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("tool test/sum failed"))
    );
}

#[test]
fn a5_phase8_tool_turn_budget_stops_infinite_loop() {
    let mut cx = phase8_cx();
    cx.grant_named("math");
    register_strict_sum_tool(&mut cx);

    let runner = fake_runner(
        &mut cx,
        vec![
            tool_call_response(vec![tool_call(
                "call-1",
                Symbol::qualified("test", "sum"),
                vec![number(1), number(1)],
            )]),
            tool_call_response(vec![tool_call(
                "call-2",
                Symbol::qualified("test", "sum"),
                vec![number(2), number(2)],
            )]),
        ],
    );
    let frame = request_frame(
        &mut cx,
        request_expr(
            "loop",
            vec![tool_descriptor(Symbol::qualified("test", "sum"))],
            Some(1),
        ),
    );
    let reply = as_component(&runner).answer(&mut cx, frame).unwrap();
    let (expr, _) = reply_expr_and_diagnostics(&mut cx, &reply);
    assert!(format!("{expr:?}").contains("tool turn budget exhausted after 1 rounds"));
}

#[test]
fn k6_10_missing_tool_capability_records_aborted_effect() {
    let mut cx = phase8_cx();
    let calls = Arc::new(AtomicUsize::new(0));
    register_counting_tool(&mut cx, calls.clone());

    let runner = fake_runner(
        &mut cx,
        vec![tool_call_response(vec![tool_call(
            "call-secret",
            Symbol::qualified("test", "secret"),
            Vec::new(),
        )])],
    );
    let frame = request_frame(
        &mut cx,
        request_expr(
            "secret",
            vec![tool_descriptor(Symbol::qualified("test", "secret"))],
            None,
        ),
    );
    let reply = as_component(&runner).answer(&mut cx, frame).unwrap();
    let (_, diagnostics) = reply_expr_and_diagnostics(&mut cx, &reply);

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("tool test/secret failed"))
    );
    assert!(
        cx.effect_ledger()
            .records()
            .iter()
            .any(|record| record.aborted)
    );
}

fn phase8_cx() -> Cx {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn fake_runner(cx: &mut Cx, script: Vec<Expr>) -> Value {
    let script_value = value_from_expr(cx, &Expr::List(script)).unwrap();
    cx.call_function(
        &Symbol::qualified("runner", "fake"),
        Args::new(vec![
            cx.factory().symbol(Symbol::new(":name")).unwrap(),
            cx.factory().symbol(Symbol::new("phase8-fake")).unwrap(),
            cx.factory().symbol(Symbol::new(":script")).unwrap(),
            script_value,
        ]),
    )
    .unwrap()
}

fn request_expr(task: &str, tools: Vec<Expr>, max_tool_rounds: Option<u32>) -> Expr {
    let mut entries = vec![
        key_bool("model-request", true),
        key_expr("task", Expr::String(task.to_owned())),
        key_expr("messages", Expr::List(Vec::new())),
        key_expr("tools", Expr::List(tools)),
    ];
    if let Some(max_tool_rounds) = max_tool_rounds {
        entries.push(key_expr(
            "budget",
            Expr::Map(vec![key_expr("max-tool-rounds", number(max_tool_rounds))]),
        ));
    }
    Expr::Map(entries)
}

fn tool_call_response(tool_calls: Vec<Expr>) -> Expr {
    let mut entries = match final_response("") {
        Expr::Map(entries) => entries,
        _ => unreachable!(),
    };
    entries.retain(|(key, _)| *key != Expr::Symbol(Symbol::new("text")));
    for (key, value) in &mut entries {
        if *key == Expr::Symbol(Symbol::new("content")) {
            *value = Expr::List(Vec::new());
        }
        if *key == Expr::Symbol(Symbol::new("stop-reason")) {
            *value = Expr::Symbol(Symbol::new("tool-call"));
        }
    }
    entries.push(key_expr("tool-calls", Expr::List(tool_calls)));
    Expr::Map(entries)
}

fn final_response(text: &str) -> Expr {
    Expr::Map(vec![
        key_bool("model-response", true),
        key_expr("runner", Expr::Symbol(Symbol::new("phase8-fake"))),
        key_expr("model", Expr::String("runner/fake".to_owned())),
        key_expr(
            "content",
            Expr::List(vec![Expr::Map(vec![
                key_expr("type", Expr::Symbol(Symbol::new("text"))),
                key_expr("text", Expr::String(text.to_owned())),
            ])]),
        ),
        key_expr("stop-reason", Expr::Symbol(Symbol::new("stop"))),
        key_expr("text", Expr::String(text.to_owned())),
    ])
}

fn tool_call(id: &str, name: Symbol, args: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        key_expr("id", Expr::String(id.to_owned())),
        key_expr("name", Expr::Symbol(name)),
        key_expr("arguments", Expr::List(args)),
    ])
}

fn tool_descriptor(name: Symbol) -> Expr {
    Expr::Map(vec![key_expr("name", Expr::Symbol(name))])
}

fn reply_expr_and_diagnostics(
    cx: &mut Cx,
    frame: &sim_lib_server::ServerFrame,
) -> (Expr, Vec<sim_kernel::Diagnostic>) {
    let reply = eval_reply_from_frame(cx, frame).unwrap();
    let expr = reply.value.object().as_expr(cx).unwrap();
    (expr, reply.diagnostics)
}

fn register_strict_sum_tool(cx: &mut Cx) -> Arc<Tool> {
    let callable = cx.factory().opaque(Arc::new(StrictSumFn)).unwrap();
    let args_shape = shape_value(
        Symbol::qualified("test", "sum-args"),
        Arc::new(ListShape::new(vec![
            Arc::new(NumberValueShape),
            Arc::new(NumberValueShape),
        ])),
    );
    let result_shape = shape_value(Symbol::qualified("test", "sum-result"), Arc::new(AnyShape));
    let tool = Arc::new(Tool {
        symbol: Symbol::qualified("test", "sum"),
        description: "sum two numbers".to_owned(),
        args_shape,
        result_shape: Some(result_shape),
        category: Symbol::new("math"),
        capabilities: vec![sim_kernel::CapabilityName::new("math")],
        function: callable,
        address: ServerAddress::Local,
        codecs: vec![Symbol::qualified("codec", "binary")],
    });
    let value = cx.factory().opaque(tool.clone()).unwrap();
    register_tool(cx, tool.clone(), value).unwrap();
    tool
}

fn register_counting_tool(cx: &mut Cx, calls: Arc<AtomicUsize>) -> Arc<Tool> {
    let callable = cx.factory().opaque(Arc::new(CountingFn { calls })).unwrap();
    let args_shape = shape_value(Symbol::qualified("test", "secret-args"), Arc::new(AnyShape));
    let result_shape = shape_value(
        Symbol::qualified("test", "secret-result"),
        Arc::new(AnyShape),
    );
    let tool = Arc::new(Tool {
        symbol: Symbol::qualified("test", "secret"),
        description: "requires a secret capability".to_owned(),
        args_shape,
        result_shape: Some(result_shape),
        category: Symbol::new("secret"),
        capabilities: vec![sim_kernel::CapabilityName::new("secret")],
        function: callable,
        address: ServerAddress::Local,
        codecs: vec![Symbol::qualified("codec", "binary")],
    });
    let value = cx.factory().opaque(tool.clone()).unwrap();
    register_tool(cx, tool.clone(), value).unwrap();
    tool
}

#[derive(Clone)]
struct StrictSumFn;

impl Object for StrictSumFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function test/sum>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for StrictSumFn {
    fn class(&self, cx: &mut Cx) -> Result<sim_kernel::ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn sim_kernel::Callable> {
        Some(self)
    }
}

impl sim_kernel::Callable for StrictSumFn {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let mut sum = 0.0;
        for value in args.values() {
            match value.object().as_expr(cx)? {
                Expr::Number(number) => {
                    sum += number.canonical.parse::<f64>().unwrap();
                }
                other => {
                    return Err(Error::Eval(format!("expected number arg, found {other:?}")));
                }
            }
        }
        cx.factory()
            .number_literal(Symbol::qualified("numbers", "f64"), sum.to_string())
    }
}

#[derive(Clone)]
struct CountingFn {
    calls: Arc<AtomicUsize>,
}

impl Object for CountingFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function test/secret>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for CountingFn {
    fn class(&self, cx: &mut Cx) -> Result<sim_kernel::ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn sim_kernel::Callable> {
        Some(self)
    }
}

impl sim_kernel::Callable for CountingFn {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        cx.factory().string("called".to_owned())
    }
}

fn number(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn key_bool(name: &str, value: bool) -> (Expr, Expr) {
    key_expr(name, Expr::Bool(value))
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
