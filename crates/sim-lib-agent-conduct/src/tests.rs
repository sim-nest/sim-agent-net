use std::sync::Arc;

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Expr, Object, Result, Symbol,
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

fn catalog_cards() -> Vec<AgentStepCard> {
    let specs: &[(&str, &[&str], &[&str])] = &[
        ("component", &["component"], &["continue", "error"]),
        ("delegate", &["delegate"], &["continue", "error"]),
        ("finish", &[], &["finished", "error"]),
        ("model-turn", &["runner"], &["tool-calls", "final", "error"]),
        ("plan", &["runner"], &["created", "error"]),
        (
            "replan",
            &["runner"],
            &["keep", "replace", "done", "stop", "error"],
        ),
        (
            "review",
            &["reviewer"],
            &["accept", "revise", "reject", "error"],
        ),
        ("stop", &[], &["stopped"]),
        ("tool-batch", &["tools"], &["continue", "final", "error"]),
    ];
    specs
        .iter()
        .map(|(id, roles, outcomes)| AgentStepCard {
            step_id: Symbol::qualified("agent.step", *id),
            input_shape: run_frame_shape(),
            output_shape: run_frame_shape(),
            roles: roles.iter().map(|role| Symbol::new(*role)).collect(),
            outcomes: outcomes
                .iter()
                .map(|outcome| Symbol::new(*outcome))
                .collect(),
            ..Default::default()
        })
        .collect()
}
fn conduct() -> AgentConduct {
    validate_agent_conduct(parse_package(PACKAGE).unwrap(), &cards()).unwrap()
}
fn conduct_context() -> Cx {
    let mut cx = sim_kernel::testing::bare_cx();
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
    let mut context = conduct_context();
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
    let context = conduct_context();
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

#[test]
fn data_catalog_loads_through_registry_and_normalizes_to_distinct_reports() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.grant(sim_lib_topology::topology_write_capability());
    let mut registry = TopologyRegistry::new();
    let entries = load_agent_conduct_catalog(&mut cx, &mut registry, &catalog_cards()).unwrap();
    assert_eq!(entries.len(), 7);
    assert_eq!(registry.list().len(), 7);
    assert!(entries.iter().all(|entry| entry.embedded_tests == 1));
    let reports = entries
        .iter()
        .map(|entry| {
            let graph = &entry.conduct.topology.graph;
            format!(
                "{}|{}|{}|{}|{:?}",
                entry.id,
                graph.nodes.len(),
                graph.edges.len(),
                graph.budget.max_steps,
                entry.conduct.required_roles
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(reports.len(), 7);
}

#[test]
fn every_catalog_package_has_embedded_contract_data() {
    assert_eq!(
        agent_conduct_catalog_sources().last().unwrap().id,
        "agent/triage-v1"
    );
    for item in agent_conduct_catalog_sources() {
        let package = parse_package(item.source).unwrap();
        for field in ["profile", "result_shape", "domain_budget", "diagram"] {
            assert!(
                package
                    .metadata
                    .iter()
                    .any(|(key, _)| key.name.as_ref() == field),
                "{} missing {field}",
                item.id
            );
        }
        assert_eq!(package.tests.len(), 1);
        assert!(package.graph.budget.max_steps > 0);
    }
    let implementation = include_str!("lib.rs");
    assert!(!implementation.contains("struct Triage"));
    assert!(!implementation.contains("fn triage"));
}

#[test]
fn catalog_bounded_edge_owns_visit_order_and_exhaustion() {
    let mut visits = Vec::new();
    let result =
        run_catalog_bounded_edge("agent/react-v1", "answer", "tools", 2, 0, |value, visit| {
            visits.push(visit);
            if visit == 2 {
                Ok(BoundedEdgeStep::Complete(value + 1))
            } else {
                Ok(BoundedEdgeStep::Continue(value + 1))
            }
        })
        .unwrap();
    assert_eq!(result, 3);
    assert_eq!(visits, vec![0, 1, 2]);

    let error =
        run_catalog_bounded_edge("agent/verify-retry-v1", "verify", "act", 1, (), |(), _| {
            Ok(BoundedEdgeStep::Continue(()))
        })
        .unwrap_err();
    assert!(error.to_string().contains("exhausted after 1 visit(s)"));
}
// conformance: topology-backed agent conduct certification and execution.
