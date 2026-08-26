use std::{collections::BTreeMap, sync::Arc};

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Expr, Object, Result, Symbol,
};
use sim_lib_agent_conduct::{AgentNodeBinding, bind_agent_conduct, validate_agent_conduct};
use sim_lib_agent_conduct_core::{AgentRunFrame, AgentStepCard};
use sim_lib_topology::parse_package;

use crate::{
    AgentStep, AgentStepFactory, AgentStepRegistry, PhaseOptions, admit_phase_tool, complete_phase,
    enter_phase, execute_checkpoint, execute_finish, execute_stop, standard_step_cards,
};

// conformance: standard and third-party agent steps remain complete, precise, and open.

use super::support::{eval_cx, install_agent_lib};

struct Uppercase;

impl AgentStep for Uppercase {
    fn execute(
        &self,
        _cx: &mut Cx,
        frame: &mut AgentRunFrame,
    ) -> Result<sim_lib_agent_conduct_core::AgentEvent> {
        let Expr::String(input) = &frame.working else {
            return Err(sim_kernel::Error::Eval("uppercase expects text".into()));
        };
        frame.working = Expr::String(input.to_ascii_uppercase());
        frame.outcome = Symbol::new("continue");
        execute_checkpoint(frame, Expr::Symbol(Symbol::new("uppercase-complete")))
    }
}

struct UppercaseFactory;

impl AgentStepFactory for UppercaseFactory {
    fn version(&self) -> u64 {
        1
    }

    fn bind(&self, _roles: &BTreeMap<Symbol, Expr>, _options: &Expr) -> Result<Arc<dyn AgentStep>> {
        Ok(Arc::new(Uppercase))
    }
}

fn uppercase_card() -> AgentStepCard {
    AgentStepCard {
        step_id: Symbol::qualified("example.step", "uppercase"),
        version: 1,
        outcomes: vec![],
        roles: vec![],
        ..Default::default()
    }
}

struct StepCallable(Arc<dyn AgentStep>);

impl Object for StepCallable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<example.step/uppercase>".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for StepCallable {
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

impl Callable for StepCallable {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<sim_kernel::Value> {
        let input = args.values()[0].object().as_expr(cx)?;
        let value = construct_citizen(cx, &input)?;
        let mut frame = value
            .object()
            .downcast_ref::<AgentRunFrame>()
            .cloned()
            .ok_or(sim_kernel::Error::TypeMismatch {
                expected: "agent/RunFrame",
                found: "non-run-frame",
            })?;
        self.0.execute(cx, &mut frame)?;
        cx.factory().opaque(Arc::new(frame))
    }
}

fn construct_citizen(cx: &mut Cx, expr: &Expr) -> Result<sim_kernel::Value> {
    let Expr::Extension { tag, payload } = expr else {
        return Err(sim_kernel::Error::TypeMismatch {
            expected: "citizen read-construct",
            found: "other expression",
        });
    };
    if *tag != Symbol::qualified("citizen", "read-construct") {
        return Err(sim_kernel::Error::TypeMismatch {
            expected: "citizen read-construct",
            found: "other extension",
        });
    }
    let Expr::Vector(items) = payload.as_ref() else {
        return Err(sim_kernel::Error::TypeMismatch {
            expected: "citizen read-construct vector",
            found: "other payload",
        });
    };
    let Some(Expr::Symbol(class)) = items.first() else {
        return Err(sim_kernel::Error::TypeMismatch {
            expected: "citizen class symbol",
            found: "other constructor head",
        });
    };
    let values = items[1..]
        .iter()
        .cloned()
        .map(|item| cx.factory().expr(item))
        .collect::<Result<Vec<_>>>()?;
    cx.read_construct(class, values)
}

#[test]
fn standard_cards_are_complete_and_precise() {
    let cards = standard_step_cards();
    let ids = cards
        .iter()
        .map(|card| card.step_id.name.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "checkpoint",
            "component",
            "delegate",
            "finish",
            "model-turn",
            "plan",
            "replan",
            "review",
            "stop",
            "tool-batch"
        ]
    );
    assert!(cards.iter().all(|card| {
        !card.outcomes.is_empty()
            && card.input_shape == card.output_shape
            && !card.replay.name.is_empty()
    }));
}

#[test]
fn phase_finish_stop_and_checkpoint_are_single_steps() {
    let mut frame = AgentRunFrame::standard(Symbol::qualified("run", "steps"), Expr::Nil);
    let phase = PhaseOptions {
        id: Symbol::qualified("phase", "draft"),
        instructions: Expr::String("draft once".into()),
        allowed_tools: vec![Symbol::qualified("tool", "read")],
    };
    enter_phase(&mut frame, &phase).unwrap();
    admit_phase_tool(&phase, &Symbol::qualified("tool", "read")).unwrap();
    assert!(admit_phase_tool(&phase, &Symbol::qualified("tool", "write")).is_err());
    complete_phase(&mut frame, &phase).unwrap();
    let mut cx = eval_cx();
    execute_finish(&mut frame, &mut cx, &[]).unwrap();
    execute_stop(
        &mut frame,
        Symbol::qualified("agent.stop", "requested"),
        Expr::String("operator".into()),
    )
    .unwrap();
    let event = execute_checkpoint(&mut frame, Expr::String("review".into())).unwrap();
    assert_eq!(event.kind, Symbol::qualified("agent.event", "checkpoint"));
}

#[test]
fn third_party_step_registers_and_runs_without_agent_or_conduct_changes() {
    let mut registry = AgentStepRegistry::new();
    registry
        .register(uppercase_card(), Arc::new(UppercaseFactory))
        .unwrap();
    let duplicate = registry.register(uppercase_card(), Arc::new(UppercaseFactory));
    assert!(duplicate.unwrap_err().to_string().contains("duplicate"));
    let mut mismatch = uppercase_card();
    mismatch.step_id = Symbol::qualified("example.step", "mismatch");
    mismatch.version = 2;
    assert!(
        registry
            .register(mismatch, Arc::new(UppercaseFactory))
            .unwrap_err()
            .to_string()
            .contains("does not match")
    );

    let step = registry
        .bind(
            &Symbol::qualified("example.step", "uppercase"),
            &BTreeMap::new(),
            &Expr::Nil,
        )
        .unwrap();
    let mut frame = AgentRunFrame::standard(
        Symbol::qualified("run", "third-party"),
        Expr::String("hello".into()),
    );
    frame.working = Expr::String("hello".into());
    step.execute(&mut eval_cx(), &mut frame).unwrap();
    assert_eq!(frame.working, Expr::String("HELLO".into()));
    assert_eq!(registry.cards(), vec![uppercase_card()]);

    let source = r#"
graph:
topology third-party-uppercase
node in verb=in output=agent/RunFrame
node upper verb=call target=example.step/uppercase
node finish verb=call target=agent.step/finish
node out verb=out input=agent/RunFrame
wire in -> upper
wire upper -> finish
wire finish -> out
budget max-steps=8 max-node-visits=4 max-edge-visits=4

metadata:
profile=agent/conduct-v1
"#;
    let finish = standard_step_cards()
        .into_iter()
        .find(|card| card.step_id == Symbol::qualified("agent.step", "finish"))
        .unwrap();
    let conduct =
        validate_agent_conduct(parse_package(source).unwrap(), &[uppercase_card(), finish])
            .unwrap();
    let mut cx = eval_cx();
    install_agent_lib(&mut cx).unwrap();
    cx.grant(sim_lib_topology::topology_run_capability());
    cx.grant(sim_kernel::read_construct_capability());
    let upper = registry
        .bind(
            &Symbol::qualified("example.step", "uppercase"),
            &BTreeMap::new(),
            &Expr::Nil,
        )
        .unwrap();
    let upper = cx.factory().opaque(Arc::new(StepCallable(upper))).unwrap();
    let finish = cx
        .factory()
        .opaque(Arc::new(StepCallable(Arc::new(Uppercase))))
        .unwrap();
    let bindings = bind_agent_conduct(
        &conduct,
        vec![
            AgentNodeBinding::new(
                "upper",
                Symbol::qualified("example.step", "uppercase"),
                upper,
            ),
            AgentNodeBinding::new("finish", Symbol::qualified("agent.step", "finish"), finish),
        ],
    )
    .unwrap();
    let mut initial = AgentRunFrame::standard(
        Symbol::qualified("run", "topology"),
        Expr::String("topology".into()),
    );
    initial.working = Expr::String("topology".into());
    let initial = cx
        .factory()
        .opaque(Arc::new(initial))
        .unwrap()
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let result = conduct.run(&mut cx, initial, bindings).unwrap();
    let result = construct_citizen(&mut cx, &result).unwrap();
    let frame = result.object().downcast_ref::<AgentRunFrame>().unwrap();
    assert_eq!(frame.working, Expr::String("TOPOLOGY".into()));
}
