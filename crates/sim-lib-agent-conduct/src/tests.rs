use std::sync::Arc;

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, DefaultFactory, Expr, NoopEvalPolicy,
    Object, Result, Symbol,
};
use sim_lib_agent_conduct_core::AgentStepCard;
use sim_lib_topology::{TopologyProgress, parse_package, topology_run_capability};

use super::*;

const PACKAGE: &str = r#"
graph:
topology echo-conduct
node in verb=in output=agent/RunFrame
node echo verb=call target=agent.step/echo role=runner
node finish verb=call target=agent.step/finish
node out verb=out input=agent/RunFrame
wire in -> echo
wire echo -> finish
wire finish -> out
budget max-steps=8 max-node-visits=4 max-edge-visits=4

metadata:
profile=agent/conduct-v1
requires-roles=[runner]
"#;

#[derive(Debug)]
struct Echo;
impl Object for Echo {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<agent.step/echo>".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl sim_kernel::ObjectCompat for Echo {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for Echo {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<sim_kernel::Value> {
        let expr = args.values()[0].object().as_expr(cx)?;
        cx.factory().expr(expr)
    }
}

fn cards() -> Vec<AgentStepCard> {
    vec![
        AgentStepCard {
            step_id: Symbol::qualified("agent.step", "echo"),
            roles: vec![Symbol::new("runner")],
            outcomes: vec![],
            ..Default::default()
        },
        AgentStepCard {
            step_id: Symbol::qualified("agent.step", "finish"),
            outcomes: vec![],
            roles: vec![],
            ..Default::default()
        },
    ]
}
fn conduct() -> AgentConduct {
    validate_agent_conduct(parse_package(PACKAGE).unwrap(), &cards()).unwrap()
}
fn cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    cx.grant(topology_run_capability());
    let predicate = cx.factory().opaque(Arc::new(Echo)).unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("agent", "outcome-continue"), predicate)
        .unwrap();
    cx
}
fn bindings(cx: &mut Cx, conduct: &AgentConduct) -> TopologyBindings {
    let echo = cx.factory().opaque(Arc::new(Echo)).unwrap();
    bind_agent_conduct(
        conduct,
        vec![
            AgentNodeBinding::new(
                "echo",
                Symbol::qualified("agent.step", "echo"),
                echo.clone(),
            ),
            AgentNodeBinding::new("finish", Symbol::qualified("agent.step", "finish"), echo),
        ],
    )
    .unwrap()
}

#[test]
fn three_node_conduct_validates_runs_pauses_resumes_reflects_and_diagrams() {
    let conduct = conduct();
    assert_eq!(conduct.required_roles, vec![Symbol::new("runner")]);
    assert_eq!(conduct.browse_summary.call_nodes, 2);
    let input = run_frame_shape();
    let mut context = cx();
    let bound = bindings(&mut context, &conduct);
    let first = conduct
        .step(&mut context, input.clone(), None, bound)
        .unwrap();
    assert_eq!(first.progress, TopologyProgress::Advanced);
    let bound = bindings(&mut context, &conduct);
    let second = conduct
        .step(&mut context, input.clone(), Some(first.continuation), bound)
        .unwrap();
    assert_eq!(second.progress, TopologyProgress::Advanced);
    let bound = bindings(&mut context, &conduct);
    let third = conduct
        .step(
            &mut context,
            input.clone(),
            Some(second.continuation),
            bound,
        )
        .unwrap();
    assert_eq!(third.progress, TopologyProgress::Advanced);
    let bound = bindings(&mut context, &conduct);
    let fourth = conduct
        .step(&mut context, input.clone(), Some(third.continuation), bound)
        .unwrap();
    assert!(matches!(fourth.progress, TopologyProgress::Output(_)));
    let bound = bindings(&mut context, &conduct);
    assert_eq!(
        conduct.run(&mut context, input, bound).unwrap(),
        run_frame_shape()
    );
    assert!(matches!(conduct.reflect(&context), Expr::Map(_)));
    assert!(matches!(conduct.diagram(&context), Expr::Map(_)));
}

#[test]
fn validation_rejects_card_role_route_terminal_and_binding_disagreement() {
    let mut wrong_roles = cards();
    wrong_roles[0].roles = vec![Symbol::new("judge")];
    assert!(
        validate_agent_conduct(parse_package(PACKAGE).unwrap(), &wrong_roles)
            .unwrap_err()
            .to_string()
            .contains("requires-roles")
    );
    let no_route = PACKAGE.replace(
        "wire echo -> finish",
        "wire echo -> finish when=agent/outcome-other",
    );
    let mut routed_cards = cards();
    routed_cards[0].outcomes = vec![Symbol::qualified("agent.outcome", "continue")];
    assert!(
        validate_agent_conduct(parse_package(&no_route).unwrap(), &routed_cards)
            .unwrap_err()
            .to_string()
            .contains("exactly one")
    );
    let direct = PACKAGE.replace("wire finish -> out", "wire echo -> out");
    assert!(validate_agent_conduct(parse_package(&direct).unwrap(), &cards()).is_err());
    let conduct = conduct();
    let context = cx();
    let echo = context.factory().opaque(Arc::new(Echo)).unwrap();
    let error = bind_agent_conduct(
        &conduct,
        vec![AgentNodeBinding::new(
            "echo",
            Symbol::qualified("agent.step", "wrong"),
            echo,
        )],
    )
    .err()
    .expect("incompatible binding rejected");
    assert!(error.to_string().contains("Card-incompatible"));
}

#[test]
fn malformed_topology_is_rejected_by_the_topology_owner_first() {
    let malformed = PACKAGE.replace("node echo verb=call", "node in verb=call");
    assert!(validate_agent_conduct(parse_package(&malformed).unwrap(), &cards()).is_err());
}

#[test]
fn dependency_and_source_guards_keep_the_adapter_narrow() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "sim-lib-agent =",
        "sim-lib-agent-runner",
        "sim-lib-bridge",
        "sim-lib-provider",
        "sim-lib-tool",
        "sim-transport-ports",
        "sim-lib-memory",
    ] {
        assert!(
            !manifest.to_ascii_lowercase().contains(forbidden),
            "forbidden dependency marker {forbidden}"
        );
    }
    let source = include_str!("lib.rs");
    for duplicate in [
        "struct Graph",
        "struct Scheduler",
        "struct TopologyRegistry",
        "std::fs",
        "std::net",
    ] {
        assert!(
            !source.contains(duplicate),
            "forbidden implementation marker {duplicate}"
        );
    }
}
