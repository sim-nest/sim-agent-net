use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use sim_kernel::{
    Args, Callable, ClassRef, Cx, Error, Expr, NumberLiteral, Object, Result, Symbol, Value,
};

use crate::{AgentComponent, AgentRole, installed_codecs};

use super::swarm::{SwarmLoopSession, SwarmRoundRecord};
use super::types::{SwarmRegistry, SwarmStatus};

pub(super) struct SwarmUntilFn;

pub(super) fn update_registry_status(
    registry: &Arc<Mutex<SwarmRegistry>>,
    session: &Arc<Mutex<SwarmLoopSession>>,
) -> Result<()> {
    let mut registry = registry
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm registry"))?;
    registry.status = session_status(session)?;
    Ok(())
}

pub(super) fn session_status(session: &Arc<Mutex<SwarmLoopSession>>) -> Result<SwarmStatus> {
    let session = session
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
    Ok(SwarmStatus {
        active: session.active,
        task_id: Some(session.task_id.clone()),
        turns_used: session.turns_used,
        turns_remaining: Some(session.max_turns.saturating_sub(session.turns_used)),
        cost_used: session.cost_used,
        cost_remaining: session
            .max_cost
            .map(|limit| (limit - session.cost_used).max(0.0)),
        budget_exhausted: session.budget_exhausted,
        last_value: session.last_value.clone(),
    })
}

pub(super) fn session_transcript_expr(session: &Arc<Mutex<SwarmLoopSession>>) -> Result<Expr> {
    let session = session
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
    Ok(Expr::List(
        session
            .transcript
            .iter()
            .map(SwarmRoundRecord::expr)
            .collect(),
    ))
}

pub(super) fn state_expr(session: &Arc<Mutex<SwarmLoopSession>>) -> Result<Expr> {
    let session = session
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("task-id")),
            Expr::String(session.task_id.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("strategy")),
            Expr::Symbol(session.planner.strategy.clone()),
        ),
        (Expr::Symbol(Symbol::new("result")), session.current.clone()),
        (
            Expr::Symbol(Symbol::new("transcript")),
            Expr::List(
                session
                    .transcript
                    .iter()
                    .map(SwarmRoundRecord::expr)
                    .collect(),
            ),
        ),
        (
            Expr::Symbol(Symbol::new("turns-used")),
            number_expr(session.turns_used),
        ),
        (
            Expr::Symbol(Symbol::new("budget-exhausted")),
            Expr::Bool(session.budget_exhausted),
        ),
        (Expr::Symbol(Symbol::new("done")), Expr::Bool(session.done)),
        (
            Expr::Symbol(Symbol::new("last-value")),
            session.last_value.clone(),
        ),
    ]))
}

pub(super) fn until_value(cx: &mut Cx) -> Result<Value> {
    cx.factory().opaque(Arc::new(SwarmUntilFn))
}

pub(super) fn number_expr(value: u32) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

pub(super) fn reply_cost(expr: &Expr) -> f64 {
    if let Expr::Map(entries) = expr {
        for (key, value) in entries {
            if *key == Expr::Symbol(Symbol::new("cost")) {
                return parse_numeric_expr(value).unwrap_or(1.0);
            }
        }
    }
    1.0
}

pub(super) fn planner_strategy(planner: Option<&Value>) -> Symbol {
    let Some(planner) = planner else {
        return Symbol::new("budget");
    };
    let Some(component) = planner.object().downcast_ref::<AgentComponent>() else {
        return Symbol::new("budget");
    };
    if let Some((_, Expr::Symbol(strategy))) = component
        .spec
        .iter()
        .find(|(key, _)| key.name.as_ref() == "strategy")
    {
        return strategy.clone();
    }
    Symbol::new("budget")
}

pub(super) fn default_member_role(index: usize) -> Symbol {
    match index % 4 {
        0 => AgentRole::Worker.as_symbol(),
        1 => AgentRole::Critic.as_symbol(),
        2 => AgentRole::Judge.as_symbol(),
        _ => AgentRole::Verifier.as_symbol(),
    }
}

pub(super) fn next_role(
    available_roles: &[Symbol],
    last_round: &[SwarmRoundRecord],
) -> Option<Symbol> {
    if available_roles.is_empty() {
        return None;
    }
    let last_role = last_round.last().map(|record| record.role.clone());
    let Some(last_role) = last_role else {
        return available_roles.first().cloned();
    };
    let index = available_roles
        .iter()
        .position(|role| role == &last_role)
        .unwrap_or(0);
    available_roles
        .get((index + 1) % available_roles.len())
        .cloned()
}

pub(super) fn first_codec_for_swarm(cx: &mut Cx) -> Symbol {
    installed_codecs(cx)
        .into_iter()
        .next()
        .unwrap_or_else(|| Symbol::qualified("codec", "binary"))
}

impl Callable for SwarmUntilFn {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let [state] = args.values() else {
            return Err(Error::Eval(
                "swarm loop until predicate expects one state".to_owned(),
            ));
        };
        let done = state_done(&state.object().as_expr(cx)?);
        cx.factory().bool(done)
    }
}

impl Object for SwarmUntilFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<swarm-until>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for SwarmUntilFn {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory()
            .class_stub(sim_kernel::ClassId(0), Symbol::qualified("swarm", "Until"))
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

fn parse_numeric_expr(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(number) => number.canonical.parse().ok(),
        Expr::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn state_done(expr: &Expr) -> bool {
    let Expr::Map(entries) = expr else {
        return false;
    };
    entries.iter().any(|(key, value)| {
        matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "done" || symbol.name.as_ref() == "budget-exhausted")
            && matches!(value, Expr::Bool(true))
    })
}
