use super::support::{
    eval_cx, flatten_text, install_agent_lib, install_roundtrip_codecs, register_sum_tool,
};
use crate::{Agent, AgentFabric, component_name};
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::Connection;

#[test]
fn a5_agent_surface_round_trips_and_agent_addresses_resolve() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");
    cx.grant_named("agent-replace");
    cx.grant_named("agent-reflect");

    let tool = register_sum_tool(&mut cx);
    let tool_value = cx.resolve_value(&tool.symbol).unwrap();
    let recorder = cx
        .call_function(
            &Symbol::qualified("recorder", "journal"),
            sim_kernel::Args::default(),
        )
        .unwrap();
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("alpha")).unwrap(),
                cx.factory().symbol(Symbol::new(":tools")).unwrap(),
                tool_value.clone(),
                cx.factory().symbol(Symbol::new(":recorders")).unwrap(),
                recorder.clone(),
            ]),
        )
        .unwrap();
    assert!(agent.object().downcast_ref::<Agent>().is_some());

    let component = cx
        .call_function(
            &Symbol::qualified("agent", "component"),
            sim_kernel::Args::new(vec![
                agent.clone(),
                cx.factory().string("tool".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    assert_eq!(component_name(&component).unwrap(), tool.symbol);

    let reflected = cx
        .call_function(
            &Symbol::qualified("agent", "reflect"),
            sim_kernel::Args::new(vec![agent.clone()]),
        )
        .unwrap();
    assert!(flatten_text(&reflected.object().as_expr(&mut cx).unwrap()).contains("alpha"));

    let persona = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("terse".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "replace"),
        sim_kernel::Args::new(vec![
            agent.clone(),
            cx.factory().string("persona".to_owned()).unwrap(),
            persona,
        ]),
    )
    .unwrap();
    cx.call_function(
        &Symbol::qualified("agent", "restart"),
        sim_kernel::Args::new(vec![agent.clone()]),
    )
    .unwrap();
    let derived = cx
        .call_function(
            &Symbol::qualified("agent", "derive"),
            sim_kernel::Args::new(vec![
                agent,
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("alpha-voice")).unwrap(),
            ]),
        )
        .unwrap();
    assert!(derived.object().downcast_ref::<Agent>().is_some());
}

#[test]
fn a6_to_a8_swarm_topology_gateway_and_wire_surfaces_work() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");
    cx.grant_named("swarm-launch");

    let tool = register_sum_tool(&mut cx);
    let tool_value = cx.resolve_value(&tool.symbol).unwrap();
    let verifier = cx
        .call_function(
            &Symbol::qualified("judge", "threshold"),
            sim_kernel::Args::default(),
        )
        .unwrap();
    let worker = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("worker")).unwrap(),
                cx.factory().symbol(Symbol::new(":tools")).unwrap(),
                tool_value,
            ]),
        )
        .unwrap();
    let _critic = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("critic")).unwrap(),
                cx.factory().symbol(Symbol::new(":judge")).unwrap(),
                verifier.clone(),
            ]),
        )
        .unwrap();

    let wire = cx
        .call_function(
            &Symbol::qualified("agent", "wire"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":steps")).unwrap(),
                worker.clone(),
            ]),
        )
        .unwrap();
    assert!(wire.object().downcast_ref::<Connection>().is_some());

    let ring = cx
        .call_function(
            &Symbol::qualified("topology", "ring"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":agents")).unwrap(),
                worker.clone(),
            ]),
        )
        .unwrap();
    assert!(ring.object().downcast_ref::<Connection>().is_some());

    let open_claw = cx
        .call_function(
            &Symbol::qualified("topology", "open-claw"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":steps")).unwrap(),
                worker.clone(),
            ]),
        )
        .unwrap();
    assert!(open_claw.object().downcast_ref::<Connection>().is_some());

    let swarm = cx
        .call_function(
            &Symbol::qualified("swarm", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("pair")).unwrap(),
                cx.factory().symbol(Symbol::new(":max-turns")).unwrap(),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "2".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    assert!(swarm.object().downcast_ref::<AgentFabric>().is_some());

    let realized = swarm
        .object()
        .as_eval_fabric()
        .expect("swarm should expose eval fabric")
        .realize(
            &mut cx,
            sim_kernel::EvalRequest {
                expr: Expr::List(vec![
                    Expr::Number(sim_kernel::NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "1".to_owned(),
                    }),
                    Expr::Number(sim_kernel::NumberLiteral {
                        domain: Symbol::qualified("numbers", "f64"),
                        canonical: "1".to_owned(),
                    }),
                ]),
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
    assert!(
        flatten_text(&realized.value.object().as_expr(&mut cx).unwrap()).contains("transcript")
    );

    let as_site = cx
        .call_function(
            &Symbol::qualified("swarm", "as-site"),
            sim_kernel::Args::new(vec![swarm.clone()]),
        )
        .unwrap();
    assert!(as_site.object().downcast_ref::<Connection>().is_some());

    let gateway = cx
        .call_function(
            &Symbol::qualified("gateway", "create"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":address")).unwrap(),
                cx.factory()
                    .expr(Expr::Symbol(Symbol::new("local")))
                    .unwrap(),
            ]),
        )
        .unwrap();
    assert!(gateway.object().downcast_ref::<Connection>().is_some());
}

#[test]
fn r20_agent_spawn_requires_capability_for_make_and_start() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let denied_make = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("denied")).unwrap(),
            ]),
        )
        .unwrap_err();
    assert!(matches!(
        denied_make,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("agent-spawn")
    ));

    cx.grant_named("agent-spawn");
    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("spawned")).unwrap(),
            ]),
        )
        .unwrap();
    let denied_start = cx
        .with_capabilities(Default::default(), |cx| {
            cx.call_function(
                &Symbol::qualified("agent", "start"),
                sim_kernel::Args::new(vec![agent.clone()]),
            )
        })
        .unwrap_err();
    assert!(matches!(
        denied_start,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("agent-spawn")
    ));
}

#[test]
fn r20_agent_replace_and_reflect_require_capabilities() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("agent-spawn");

    let agent = cx
        .call_function(
            &Symbol::qualified("agent", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("guarded")).unwrap(),
            ]),
        )
        .unwrap();
    let persona = cx
        .call_function(
            &Symbol::qualified("persona", "style"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":voice")).unwrap(),
                cx.factory().string("plain".to_owned()).unwrap(),
            ]),
        )
        .unwrap();

    let denied_reflect = cx
        .call_function(
            &Symbol::qualified("agent", "reflect"),
            sim_kernel::Args::new(vec![agent.clone()]),
        )
        .unwrap_err();
    assert!(matches!(
        denied_reflect,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("agent-reflect")
    ));

    let denied_replace = cx
        .call_function(
            &Symbol::qualified("agent", "replace"),
            sim_kernel::Args::new(vec![
                agent.clone(),
                cx.factory().string("persona".to_owned()).unwrap(),
                persona.clone(),
            ]),
        )
        .unwrap_err();
    assert!(matches!(
        denied_replace,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("agent-replace")
    ));

    cx.grant_named("agent-reflect");
    cx.grant_named("agent-replace");
    let reflected = cx
        .call_function(
            &Symbol::qualified("agent", "reflect"),
            sim_kernel::Args::new(vec![agent.clone()]),
        )
        .unwrap();
    assert!(flatten_text(&reflected.object().as_expr(&mut cx).unwrap()).contains("guarded"));
    cx.call_function(
        &Symbol::qualified("agent", "replace"),
        sim_kernel::Args::new(vec![
            agent,
            cx.factory().string("persona".to_owned()).unwrap(),
            persona,
        ]),
    )
    .unwrap();
}

#[test]
fn r20_swarm_launch_requires_capability_for_surface_and_fabric() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let swarm = cx
        .call_function(
            &Symbol::qualified("swarm", "make"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":name")).unwrap(),
                cx.factory().symbol(Symbol::new("guarded-swarm")).unwrap(),
            ]),
        )
        .unwrap();

    let denied_launch = cx
        .call_function(
            &Symbol::qualified("swarm", "launch"),
            sim_kernel::Args::new(vec![swarm.clone(), cx.factory().expr(Expr::Nil).unwrap()]),
        )
        .unwrap_err();
    assert!(matches!(
        denied_launch,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("swarm-launch")
    ));

    let denied_realize = match swarm.object().as_eval_fabric().unwrap().realize(
        &mut cx,
        sim_kernel::EvalRequest {
            expr: Expr::Nil,
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
    ) {
        Ok(_) => panic!("swarm fabric realize should require swarm-launch"),
        Err(error) => error,
    };
    assert!(matches!(
        denied_realize,
        Error::CapabilityDenied { capability }
            if capability == sim_kernel::CapabilityName::new("swarm-launch")
    ));

    cx.grant_named("swarm-launch");
    let launched = cx
        .call_function(
            &Symbol::qualified("swarm", "launch"),
            sim_kernel::Args::new(vec![swarm, cx.factory().expr(Expr::Nil).unwrap()]),
        )
        .unwrap();
    assert!(flatten_text(&launched.object().as_expr(&mut cx).unwrap()).contains("transcript"));
}
