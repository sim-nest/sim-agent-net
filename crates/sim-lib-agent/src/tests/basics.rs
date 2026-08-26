use super::support::{DummyComponent, eval_cx};
use crate::{
    AGENT_LIB_ID, AgentLib, AgentRole, Component, ComponentKind, install_agent_lib,
    net_http_capability, stamp_envelope_role, stamp_frame_role,
};
use sim_kernel::{Args, Cx, Expr, Lib, Symbol, Value};
use sim_lib_server::{FrameEnvelope, FrameKind, ServerFrame};

#[test]
fn agent_lib_installs_and_is_idempotent() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    install_agent_lib(&mut cx).unwrap();
    assert!(cx.registry().lib(&Symbol::new("server")).is_some());
    assert!(cx.registry().lib(&Symbol::new(AGENT_LIB_ID)).is_some());

    let manifest = AgentLib.manifest();
    assert_eq!(manifest.id, Symbol::new(AGENT_LIB_ID));
}

#[test]
fn agent_lib_claims_loaded_cli_agent_entrypoint() {
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    let symbol = Symbol::qualified("cli", "main/agent");
    let envelope = cli_envelope(&mut cx, "agent", &["agent", "conduct", "list"]);

    let value = cx
        .call_function(&symbol, Args::new(vec![envelope]))
        .unwrap();

    assert!(value.object().truth(&mut cx).unwrap());
}

#[test]
fn component_contract_exposes_kind_name_capabilities_and_reflection() {
    let mut cx = eval_cx();
    let component = DummyComponent::new();
    assert_eq!(component.kind(), ComponentKind::Tool);
    assert_eq!(component.name(), &Symbol::qualified("test", "component"));
    assert_eq!(component.capabilities(), [net_http_capability()]);
    assert_eq!(
        component.reflect(&mut cx).unwrap(),
        Expr::Symbol(Symbol::qualified("test", "component"))
    );
}

#[test]
fn role_stamping_sets_role_and_increments_hop() {
    let mut envelope = FrameEnvelope {
        hop: 3,
        ..FrameEnvelope::default()
    };
    stamp_envelope_role(&mut envelope, &AgentRole::Judge);
    assert_eq!(envelope.role, Some(Symbol::new("judge")));
    assert_eq!(envelope.hop, 4);
}

#[test]
fn frame_role_stamping_saturates_hop_growth() {
    let mut frame = ServerFrame::new(
        Symbol::qualified("codec", "binary"),
        FrameKind::Notify,
        FrameEnvelope {
            hop: u32::MAX,
            ..FrameEnvelope::default()
        },
        Vec::new(),
    );
    stamp_frame_role(&mut frame, &AgentRole::Custom(Symbol::new("auditor")));
    assert_eq!(frame.envelope.role, Some(Symbol::new("auditor")));
    assert_eq!(frame.envelope.hop, u32::MAX);
}

fn cli_envelope(cx: &mut Cx, verb: &str, args: &[&str]) -> Value {
    let verb = cx.factory().string(verb.to_owned()).unwrap();
    let args = cx
        .factory()
        .list(
            args.iter()
                .map(|arg| cx.factory().string((*arg).to_owned()).unwrap())
                .collect(),
        )
        .unwrap();
    cx.factory()
        .table(vec![
            (Symbol::new("verb"), verb),
            (Symbol::new("args"), args),
        ])
        .unwrap()
}
