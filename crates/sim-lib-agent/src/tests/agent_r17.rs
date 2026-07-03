use super::support::{eval_cx, install_agent_lib, install_roundtrip_codecs};
use sim_kernel::{Expr, Symbol};
use sim_lib_server::{
    Connection, EvalSite, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_reply,
};
use std::sync::Arc;

#[derive(Clone)]
struct AppendSite {
    label: &'static str,
}

impl EvalSite for AppendSite {
    fn site_kind(&self) -> &'static str {
        "test-append"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: std::sync::OnceLock<ServerAddress> = std::sync::OnceLock::new();
        LOCAL.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        let consistency = frame.envelope.consistency;
        let request = eval_request_from_frame(cx, &frame)?;
        let mut items = match request.expr {
            Expr::List(items) => items,
            Expr::Nil => Vec::new(),
            expr => vec![expr],
        };
        items.push(Expr::String(self.label.to_owned()));
        let value = cx.factory().expr(Expr::List(items))?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn tagged_connection(cx: &mut sim_kernel::Cx, role: &str, label: &'static str) -> Connection {
    Connection::with_session(
        ServerAddress::Local,
        Symbol::qualified("codec", "lisp"),
        crate::installed_codecs(cx),
        Arc::new(AppendSite { label }),
        Some(Symbol::new(role)),
        sim_lib_server::IsolationPolicy::default(),
    )
    .unwrap()
}

#[test]
fn r17_swarm_budget_stops_after_two_turns_and_records_transcript() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("swarm-launch");

    let worker_conn = tagged_connection(&mut cx, "worker", "worker");
    let critic_conn = tagged_connection(&mut cx, "critic", "critic");
    let worker = cx.factory().opaque(Arc::new(worker_conn)).unwrap();
    let critic = cx.factory().opaque(Arc::new(critic_conn)).unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "worker"), worker.clone())
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "critic"), critic.clone())
        .unwrap();
    let blackboard = cx
        .call_function(
            &Symbol::qualified("memory", "blackboard"),
            sim_kernel::Args::new(vec![cx.factory().string("r17-board".to_owned()).unwrap()]),
        )
        .unwrap();

    let swarm = cx
        .call_function(
            &Symbol::qualified("swarm", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":agents")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::Symbol(Symbol::qualified("test", "worker")),
                        Expr::Symbol(Symbol::qualified("test", "critic")),
                    ]))
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":max-turns")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "2".to_owned())
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":blackboard")).unwrap(),
                blackboard.clone(),
            ]),
        )
        .unwrap();

    let launched = cx
        .call_function(
            &Symbol::qualified("swarm", "launch"),
            sim_kernel::Args::new(vec![
                swarm.clone(),
                cx.factory().expr(Expr::List(Vec::new())).unwrap(),
            ]),
        )
        .unwrap();
    let launched_expr = launched.object().as_expr(&mut cx).unwrap();
    let launched_text = format!("{launched_expr:?}");
    assert!(launched_text.contains("budget-exhausted") || launched_text.contains("transcript"));

    let status = cx
        .call_function(
            &Symbol::qualified("swarm", "status"),
            sim_kernel::Args::new(vec![swarm.clone()]),
        )
        .unwrap();
    let status_expr = status.object().as_expr(&mut cx).unwrap();
    let status_text = format!("{status_expr:?}");
    assert!(status_text.contains("turns-used"));
    assert!(status_text.contains("2"));

    let explain = cx
        .call_function(
            &Symbol::qualified("swarm", "explain"),
            sim_kernel::Args::new(vec![swarm.clone()]),
        )
        .unwrap();
    let explain_text = format!("{:?}", explain.object().as_expr(&mut cx).unwrap());
    assert!(explain_text.contains("worker"));
    assert!(explain_text.contains("critic"));

    let blackboard_entries = cx
        .call_function(
            &Symbol::qualified("memory", "scan"),
            sim_kernel::Args::new(vec![blackboard]),
        )
        .unwrap();
    let board_text = format!(
        "{:?}",
        blackboard_entries.object().as_expr(&mut cx).unwrap()
    );
    assert!(board_text.contains("turn"));
}

#[test]
fn r17_realize_fabric_uses_same_loop_runtime() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("swarm-launch");

    let worker_conn = tagged_connection(&mut cx, "worker", "alpha");
    let worker = cx.factory().opaque(Arc::new(worker_conn)).unwrap();
    let swarm = cx
        .call_function(
            &Symbol::qualified("swarm", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":agents")).unwrap(),
                worker,
                cx.factory().symbol(Symbol::new(":max-turns")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "1".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let fabric = swarm.object().as_eval_fabric().unwrap();
    let reply = fabric
        .realize(
            &mut cx,
            sim_kernel::EvalRequest {
                expr: Expr::List(vec![Expr::String("seed".to_owned())]),
                mode: sim_kernel::EvalMode::Eval,
                result_shape: None,
                answer_limit: None,
                stream_buffer: None,
                stream: false,
                required_capabilities: Vec::new(),
                deadline: None,
                consistency: sim_kernel::Consistency::LocalFirst,
                trace: false,
            },
        )
        .unwrap();
    let text = format!("{:?}", reply.value.object().as_expr(&mut cx).unwrap());
    assert!(text.contains("transcript"));
    assert!(text.contains("alpha"));
}
