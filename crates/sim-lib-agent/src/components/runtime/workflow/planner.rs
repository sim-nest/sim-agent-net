use crate::components::model::{AgentComponent, PlannerBackend, number_expr, number_expr_from_f64};
use crate::util::expr_to_value;
use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_server::{FrameKind, ServerFrame, eval_request_from_frame};

pub(in crate::components) fn answer_planner(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &PlannerBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "{} only answers request frames",
            component.symbol
        )));
    }
    let consistency = frame.envelope.consistency;
    let request = eval_request_from_frame(cx, &frame)?;
    let plan = planner_plan_expr(backend, request.expr);
    let value = expr_to_value(cx, &plan)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
}

fn planner_plan_expr(backend: &PlannerBackend, goal: Expr) -> Expr {
    let steps = match &goal {
        Expr::List(items) | Expr::Vector(items) => items.clone(),
        Expr::Nil => Vec::new(),
        expr => vec![expr.clone()],
    };
    let strategy = match backend {
        PlannerBackend::Budget { .. } => Symbol::new("budget"),
        PlannerBackend::Refine => Symbol::new("refine"),
        PlannerBackend::Parallel { .. } => Symbol::new("parallel"),
        PlannerBackend::Chain => Symbol::new("chain"),
    };
    let mut entries = vec![
        (
            Expr::Symbol(Symbol::new("strategy")),
            Expr::Symbol(strategy),
        ),
        (Expr::Symbol(Symbol::new("goal")), goal),
        (Expr::Symbol(Symbol::new("steps")), Expr::List(steps)),
    ];
    match backend {
        PlannerBackend::Budget {
            max_turns,
            max_cost,
        } => {
            entries.push((
                Expr::Symbol(Symbol::new("budget")),
                Expr::Map(vec![
                    (
                        Expr::Symbol(Symbol::new("max-turns")),
                        max_turns.map(number_expr).unwrap_or(Expr::Nil),
                    ),
                    (
                        Expr::Symbol(Symbol::new("max-cost")),
                        max_cost.map(number_expr_from_f64).unwrap_or(Expr::Nil),
                    ),
                ]),
            ));
        }
        PlannerBackend::Parallel { branches } => {
            entries.push((
                Expr::Symbol(Symbol::new("branches")),
                number_expr(*branches),
            ));
        }
        PlannerBackend::Refine | PlannerBackend::Chain => {}
    }
    Expr::Map(entries)
}
