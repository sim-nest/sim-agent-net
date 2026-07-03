use std::{
    any::Any,
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use sim_kernel::{Args, Consistency, Cx, EvalRequest, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_server::{
    EvalSite, ReplDriverSpec, Server, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_request,
};

use crate::{Agent, AgentManifest};

use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs, register_sum_tool};

#[test]
fn r19_attach_audit_and_trace_capture_real_stage_entries() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant(sim_kernel::CapabilityName::new("math"));

    let sum_tool = register_sum_tool(&mut cx);
    let tool = cx.factory().opaque(sum_tool).unwrap();
    let codecs = crate::installed_codecs(&cx);
    let agent = cx
        .factory()
        .opaque(Arc::new(Agent::new(
            Symbol::new("auditor"),
            AgentManifest {
                tools: vec![tool],
                ..AgentManifest::default()
            },
            Vec::new(),
            sim_lib_server::IsolationPolicy::default(),
            codecs,
        )))
        .unwrap();
    let journal = call(
        &mut cx,
        &Symbol::qualified("recorder", "journal"),
        Vec::new(),
    );
    let recorder_slot = cx.factory().symbol(Symbol::new("recorder")).unwrap();
    call(
        &mut cx,
        &Symbol::qualified("agent", "attach"),
        vec![agent.clone(), recorder_slot, journal],
    );

    let task_expr = cx
        .factory()
        .expr(Expr::List(vec![number_expr("2"), number_expr("4")]))
        .unwrap();
    let result = call(
        &mut cx,
        &Symbol::qualified("agent", "call"),
        vec![agent.clone(), task_expr],
    );
    assert_eq!(result.object().as_expr(&mut cx).unwrap(), number_expr("6"));

    let audit = call(
        &mut cx,
        &Symbol::qualified("agent", "audit"),
        vec![agent.clone()],
    );
    let audit_expr = audit.object().as_expr(&mut cx).unwrap();
    let entries = list_items(&audit_expr);
    assert!(entries.len() >= 4);

    let role_key = cx.factory().symbol(Symbol::new(":role")).unwrap();
    let role_value = cx.factory().symbol(Symbol::new("tool")).unwrap();
    let by_role = call(
        &mut cx,
        &Symbol::qualified("agent", "audit"),
        vec![agent.clone(), role_key, role_value],
    );
    for entry in list_items(&by_role.object().as_expr(&mut cx).unwrap()) {
        assert_eq!(map_symbol_field(entry, "role"), Some(Symbol::new("tool")));
    }

    let tool_key = cx.factory().symbol(Symbol::new(":tool")).unwrap();
    let tool_value = cx
        .factory()
        .symbol(Symbol::qualified("test", "sum"))
        .unwrap();
    let by_tool = call(
        &mut cx,
        &Symbol::qualified("agent", "audit"),
        vec![agent.clone(), tool_key, tool_value],
    );
    let by_tool_expr = by_tool.object().as_expr(&mut cx).unwrap();
    let tool_entries = list_items(&by_tool_expr);
    assert!(!tool_entries.is_empty());
    let task_id = map_string_field(&tool_entries[0], "task-id").unwrap();

    let trace_task = cx.factory().string(task_id.to_owned()).unwrap();
    let trace = call(
        &mut cx,
        &Symbol::qualified("agent", "trace"),
        vec![trace_task],
    );
    let trace_expr = trace.object().as_expr(&mut cx).unwrap();
    let trace_entries = list_items(&trace_expr);
    assert!(trace_entries.len() >= 2);
    assert_eq!(
        map_symbol_field(&trace_entries[0], "phase"),
        Some(Symbol::new("before"))
    );
    assert_eq!(
        map_symbol_field(&trace_entries[trace_entries.len() - 1], "phase"),
        Some(Symbol::new("after"))
    );

    let second_task_expr = cx
        .factory()
        .expr(Expr::List(vec![number_expr("1"), number_expr("3")]))
        .unwrap();
    call(
        &mut cx,
        &Symbol::qualified("agent", "call"),
        vec![agent.clone(), second_task_expr],
    );
    let audit_again = call(
        &mut cx,
        &Symbol::qualified("agent", "audit"),
        vec![agent.clone()],
    );
    let unique_tasks = list_items(&audit_again.object().as_expr(&mut cx).unwrap())
        .iter()
        .filter_map(|entry| map_string_field(entry, "task-id"))
        .collect::<BTreeSet<_>>();
    assert!(unique_tasks.len() >= 2);
}

#[test]
fn r19_trace_is_backed_by_recorder_journals() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let codecs = crate::installed_codecs(&cx);
    let agent = cx
        .factory()
        .opaque(Arc::new(Agent::new(
            Symbol::new("unrecorded"),
            AgentManifest::default(),
            Vec::new(),
            sim_lib_server::IsolationPolicy::default(),
            codecs,
        )))
        .unwrap();
    let agent_ref = agent.object().downcast_ref::<Agent>().unwrap();
    let site = agent_ref.site().unwrap();
    let task_id = "task-900000001";
    let mut frame = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "lisp"),
        EvalRequest {
            expr: Expr::String("probe".to_owned()),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::LocalFirst,
            trace: false,
        },
    )
    .unwrap();
    frame.msg_id = Some(900000001);
    site.answer(&mut cx, frame).unwrap();

    let trace_task = cx.factory().string(task_id.to_owned()).unwrap();
    let trace = call(
        &mut cx,
        &Symbol::qualified("agent", "trace"),
        vec![trace_task],
    );
    let trace_expr = trace.object().as_expr(&mut cx).unwrap();
    assert!(list_items(&trace_expr).is_empty());
}

#[test]
fn r19_agent_server_returns_backing_server_and_trigger_accepts_it() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant(sim_kernel::CapabilityName::new("cron-schedule"));

    let codecs = crate::installed_codecs(&cx);
    let agent = cx
        .factory()
        .opaque(Arc::new(Agent::new(
            Symbol::new("mailer"),
            AgentManifest::default(),
            Vec::new(),
            sim_lib_server::IsolationPolicy::default(),
            codecs,
        )))
        .unwrap();
    let server = call(&mut cx, &Symbol::qualified("agent", "server"), vec![agent]);
    let server_ref = server.object().downcast_ref::<Server>().unwrap();
    assert_eq!(server_ref.address(), &ServerAddress::Local);
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "server"), server.clone())
        .unwrap();

    let trigger_fn = cx
        .resolve_function(&Symbol::qualified("server", "trigger"))
        .unwrap();
    let trigger = trigger_fn
        .object()
        .as_callable()
        .unwrap()
        .call_exprs(
            &mut cx,
            sim_kernel::RawArgs::new(vec![
                Expr::Symbol(Symbol::qualified("test", "server")),
                Expr::Symbol(Symbol::new(":source")),
                Expr::Quote {
                    mode: sim_kernel::QuoteMode::Quote,
                    expr: Box::new(Expr::List(vec![
                        Expr::Symbol(Symbol::new("cron")),
                        Expr::Symbol(Symbol::new(":spec")),
                        Expr::String("0 * * * *".to_owned()),
                    ])),
                },
                Expr::Symbol(Symbol::new(":decode")),
                Expr::Quote {
                    mode: sim_kernel::QuoteMode::Quote,
                    expr: Box::new(Expr::Symbol(Symbol::qualified("codec", "lisp"))),
                },
            ]),
        )
        .unwrap();
    assert!(
        trigger
            .object()
            .display(&mut cx)
            .unwrap()
            .contains("trigger")
    );
    server_ref.stop_triggers().unwrap();
}

#[test]
fn r19_agent_line_driver_uses_live_agent_values() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant(sim_kernel::eval_fabric_capability());
    cx.grant(sim_kernel::CapabilityName::new("agent-drive"));

    let agent = Arc::new(Agent::new(
        Symbol::new("dev"),
        AgentManifest::default(),
        Vec::new(),
        sim_lib_server::IsolationPolicy::default(),
        crate::installed_codecs(&cx),
    ));
    agent.state.lock().unwrap().runtime_site = Arc::new(DriverSite::default());
    let agent_value = cx.factory().opaque(agent).unwrap();
    cx.registry_mut()
        .register_value(Symbol::new("dev"), agent_value)
        .unwrap();

    let spec = ReplDriverSpec::from_expr(&Expr::List(vec![
        Expr::Symbol(Symbol::new("agent")),
        Expr::Symbol(Symbol::new("dev")),
    ]))
    .unwrap();
    let mut driver = spec.create_driver(&mut cx).unwrap();
    assert_eq!(
        driver.read_line(&mut cx, "sim> ").unwrap(),
        Some("42".to_owned())
    );
    driver.write_output(&mut cx, "42\n").unwrap();
    assert_eq!(driver.read_line(&mut cx, "sim> ").unwrap(), None);
}

#[derive(Default)]
struct DriverSite {
    done: Mutex<bool>,
}

impl EvalSite for DriverSite {
    fn site_kind(&self) -> &'static str {
        "driver-agent"
    }
    fn address(&self) -> &ServerAddress {
        static ADDRESS: ServerAddress = ServerAddress::Local;
        &ADDRESS
    }
    fn codecs(&self) -> &[Symbol] {
        static CODECS: std::sync::OnceLock<Vec<Symbol>> = std::sync::OnceLock::new();
        CODECS.get_or_init(|| vec![Symbol::qualified("codec", "lisp")])
    }
    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let request = eval_request_from_frame(cx, &frame)?;
        let output = map_string_field(&request.expr, "output").unwrap_or_default();
        let reply = if *self.done.lock().unwrap() || output.contains("42") {
            Expr::Map(vec![(Expr::Symbol(Symbol::new("done")), Expr::Bool(true))])
        } else {
            *self.done.lock().unwrap() = true;
            Expr::String("42".to_owned())
        };
        sim_lib_server::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value: cx.factory().expr(reply)?,
                diagnostics: Vec::new(),
                trace: None,
            },
            frame.envelope.consistency,
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn call(cx: &mut Cx, symbol: &Symbol, args: Vec<Value>) -> Value {
    cx.call_value(cx.resolve_function(symbol).unwrap(), Args::new(args))
        .unwrap()
}

fn list_items(expr: &Expr) -> &[Expr] {
    match expr {
        Expr::List(items) | Expr::Vector(items) => items,
        other => panic!("expected list, found {other:?}"),
    }
}

fn map_symbol_field(expr: &Expr, key: &str) -> Option<Symbol> {
    match map_field(expr, key) {
        Some(Expr::Symbol(symbol)) => Some(symbol.clone()),
        _ => None,
    }
}

fn map_string_field(expr: &Expr, key: &str) -> Option<String> {
    match map_field(expr, key) {
        Some(Expr::String(text)) => Some(text.clone()),
        _ => None,
    }
}

fn map_field<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn number_expr(value: &str) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_owned(),
    })
}
