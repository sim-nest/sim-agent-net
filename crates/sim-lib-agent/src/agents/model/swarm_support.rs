use sim_kernel::{Expr, Symbol};

use crate::AgentRole;

use super::swarm::SwarmRoundRecord;

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

fn parse_numeric_expr(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(number) => number.canonical.parse().ok(),
        Expr::String(text) => text.parse().ok(),
        _ => None,
    }
}
