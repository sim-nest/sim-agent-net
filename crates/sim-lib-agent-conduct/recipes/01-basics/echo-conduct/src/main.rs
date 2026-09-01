use std::sync::Arc;

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, DefaultFactory, Expr, NoopEvalPolicy,
    Object, Result, Symbol,
};
use sim_lib_agent_conduct::{AgentNodeBinding, bind_agent_conduct, validate_agent_conduct};
use sim_lib_agent_conduct_core::{AgentRunFrame, AgentStepCard};
use sim_lib_topology::parse_package;

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

fn main() {
    let package = parse_package(include_str!("conduct.simtopo")).unwrap();
    let card = AgentStepCard {
        step_id: Symbol::qualified("agent.step", "echo"),
        roles: vec![Symbol::new("runner")],
        outcomes: vec![],
        ..Default::default()
    };
    let finish = AgentStepCard {
        step_id: Symbol::qualified("agent.step", "finish"),
        outcomes: vec![],
        roles: vec![],
        ..Default::default()
    };
    let conduct = validate_agent_conduct(package, &[card, finish]).unwrap();
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    cx.grant(sim_lib_topology::topology_run_capability());
    let predicate = cx.factory().opaque(Arc::new(Echo)).unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("agent", "outcome-continue"), predicate)
        .unwrap();
    let echo = cx.factory().opaque(Arc::new(Echo)).unwrap();
    let bindings = bind_agent_conduct(
        &conduct,
        vec![
            AgentNodeBinding::new(
                "echo",
                Symbol::qualified("agent.step", "echo"),
                echo.clone(),
            ),
            AgentNodeBinding::new("finish", Symbol::qualified("agent.step", "finish"), echo),
        ],
    )
    .unwrap();
    let _frame = AgentRunFrame::standard(Symbol::qualified("run", "recipe"), Expr::Nil);
    let output = conduct
        .run(
            &mut cx,
            Expr::Symbol(Symbol::qualified("agent", "RunFrame")),
            bindings,
        )
        .unwrap();
    assert_ne!(output, Expr::Nil);
    println!("completed");
}
