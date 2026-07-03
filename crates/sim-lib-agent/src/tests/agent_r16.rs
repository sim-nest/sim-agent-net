// Test spies carry a nested Arc<Mutex<Vec<...>>> capture buffer; the explicit type
// documents exactly what each test observes.
#![allow(clippy::type_complexity)]

use super::support::{eval_cx, install_agent_lib, install_test_codec, register_sum_tool};
use crate::Agent;
use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{
    Connection, EvalSite, FrameKind, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_reply,
};
use std::any::Any;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct RecordingSite {
    seen: Arc<Mutex<Vec<(Option<Symbol>, u32)>>>,
}

impl EvalSite for RecordingSite {
    fn site_kind(&self) -> &'static str {
        "recording"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: ServerAddress = ServerAddress::Local;
        &LOCAL
    }

    fn codecs(&self) -> &[Symbol] {
        static CODECS: std::sync::OnceLock<Vec<Symbol>> = std::sync::OnceLock::new();
        CODECS.get_or_init(|| vec![Symbol::qualified("codec", "binary")])
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        self.seen
            .lock()
            .map_err(|_| Error::PoisonedLock("recording site"))?
            .push((frame.envelope.role.clone(), frame.envelope.hop));
        let consistency = frame.envelope.consistency;
        let reply_codec = frame.codec.clone();
        let request = eval_request_from_frame(cx, &frame)?;
        let value = crate::expr_to_value(cx, &request.expr)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &reply_codec,
            sim_kernel::EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn connection_value(
    cx: &mut Cx,
    role: &str,
    seen: Arc<Mutex<Vec<(Option<Symbol>, u32)>>>,
) -> Value {
    let connection = Connection::with_session(
        ServerAddress::Local,
        Symbol::qualified("codec", "binary"),
        vec![Symbol::qualified("codec", "binary")],
        Arc::new(RecordingSite { seen }),
        Some(Symbol::new(role)),
        sim_lib_server::IsolationPolicy::default(),
    )
    .unwrap();
    cx.factory().opaque(Arc::new(connection)).unwrap()
}

#[test]
fn agent_start_builds_role_threaded_pipeline_and_stamps_hops() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");

    let retriever_seen = Arc::new(Mutex::new(Vec::new()));
    let planner_seen = Arc::new(Mutex::new(Vec::new()));
    let retriever = connection_value(&mut cx, "retriever", retriever_seen.clone());
    let planner = connection_value(&mut cx, "planner", planner_seen.clone());

    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("threaded")).unwrap(),
                cx.factory().symbol(Symbol::new(":retrievers")).unwrap(),
                retriever,
                cx.factory().symbol(Symbol::new(":planner")).unwrap(),
                planner,
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "start"),
        Args::new(vec![agent.clone()]),
    )
    .unwrap();
    let reply = cx
        .call_function(
            &Symbol::qualified("agent", "call"),
            Args::new(vec![
                agent,
                cx.factory().string("hello pipeline".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    assert_eq!(
        reply.object().as_expr(&mut cx).unwrap(),
        Expr::String("hello pipeline".to_owned())
    );
    assert_eq!(
        retriever_seen.lock().unwrap().as_slice(),
        &[(Some(Symbol::new("retriever")), 1)]
    );
    assert_eq!(
        planner_seen.lock().unwrap().as_slice(),
        &[(Some(Symbol::new("planner")), 2)]
    );
}

#[test]
fn agent_restart_rebuilds_runtime_and_preserves_memory_for_live_connections() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");
    cx.grant_named("agent-replace");

    let memory = cx
        .call_function(&Symbol::qualified("memory", "working"), Args::default())
        .unwrap();
    cx.call_function(
        &Symbol::qualified("memory", "append"),
        Args::new(vec![
            memory.clone(),
            cx.factory().string("remember this".to_owned()).unwrap(),
        ]),
    )
    .unwrap();
    let persona = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("terse".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("restartable")).unwrap(),
                cx.factory().symbol(Symbol::new(":memories")).unwrap(),
                memory.clone(),
                cx.factory().symbol(Symbol::new(":persona")).unwrap(),
                persona,
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "start"),
        Args::new(vec![
            agent.clone(),
            cx.factory().symbol(Symbol::new(":address")).unwrap(),
            cx.factory()
                .expr(Expr::List(vec![
                    Expr::Symbol(Symbol::new("agent")),
                    Expr::String("restartable".to_owned()),
                ]))
                .unwrap(),
        ]),
    )
    .unwrap();

    let connection = cx
        .call_function(
            &Symbol::qualified("agent", "connect"),
            Args::new(vec![agent.clone()]),
        )
        .unwrap();
    let before = connection
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(
            &mut cx,
            Expr::String("hello there".to_owned()),
            None,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        before.object().as_expr(&mut cx).unwrap(),
        Expr::String("hello there.".to_owned())
    );

    let replacement = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("playful".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "replace"),
        Args::new(vec![
            agent.clone(),
            cx.factory().string("persona".to_owned()).unwrap(),
            replacement,
        ]),
    )
    .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "restart"),
        Args::new(vec![agent]),
    )
    .unwrap();

    let after = connection
        .object()
        .downcast_ref::<Connection>()
        .unwrap()
        .request(
            &mut cx,
            Expr::String("hello there".to_owned()),
            None,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        after.object().as_expr(&mut cx).unwrap(),
        Expr::String("hello there".to_owned())
    );
    let recent = cx
        .call_function(
            &Symbol::qualified("memory", "recent"),
            Args::new(vec![
                memory,
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "1".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    assert_eq!(
        recent.object().as_expr(&mut cx).unwrap(),
        Expr::List(vec![Expr::String("remember this".to_owned())])
    );
}

#[test]
fn agent_derive_shares_memories_and_tools_but_isolates_persona() {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");

    let tool = register_sum_tool(&mut cx);
    let tool_value = cx.resolve_value(&tool.symbol).unwrap();
    let memory = cx
        .call_function(&Symbol::qualified("memory", "working"), Args::default())
        .unwrap();
    let persona = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("terse".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("source")).unwrap(),
                cx.factory().symbol(Symbol::new(":tools")).unwrap(),
                tool_value,
                cx.factory().symbol(Symbol::new(":memories")).unwrap(),
                memory,
                cx.factory().symbol(Symbol::new(":persona")).unwrap(),
                persona,
            ]),
        )
        .unwrap();
    let derived_persona = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("playful".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let derived = cx
        .call_function(
            &Symbol::qualified("agent", "derive"),
            Args::new(vec![
                agent.clone(),
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("fork")).unwrap(),
                cx.factory().symbol(Symbol::new(":persona")).unwrap(),
                derived_persona,
            ]),
        )
        .unwrap();

    let source_memory = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            Args::new(vec![
                agent.clone(),
                cx.factory().string("memory".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let derived_memory = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            Args::new(vec![
                derived.clone(),
                cx.factory().string("memory".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let source_tool = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            Args::new(vec![
                agent.clone(),
                cx.factory().string("tool".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let derived_tool = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            Args::new(vec![
                derived.clone(),
                cx.factory().string("tool".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let source_persona = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            Args::new(vec![
                agent,
                cx.factory().string("persona".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let derived_persona = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            Args::new(vec![
                derived,
                cx.factory().string("persona".to_owned()).unwrap(),
            ]),
        )
        .unwrap();

    assert!(std::ptr::eq(
        source_memory.object().as_any(),
        derived_memory.object().as_any(),
    ));
    assert!(std::ptr::eq(
        source_tool.object().as_any(),
        derived_tool.object().as_any(),
    ));
    assert!(!std::ptr::eq(
        source_persona.object().as_any(),
        derived_persona.object().as_any(),
    ));
    assert_ne!(
        source_persona.object().as_expr(&mut cx).unwrap(),
        derived_persona.object().as_expr(&mut cx).unwrap()
    );
    assert!(
        Agent::new(
            Symbol::new("check"),
            crate::AgentManifest::default(),
            Vec::new(),
            sim_lib_server::IsolationPolicy::default(),
            vec![Symbol::qualified("codec", "binary")],
        )
        .site()
        .is_ok()
    );
    assert_eq!(FrameKind::Request.as_symbol(), Symbol::new("request"));
}
