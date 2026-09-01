use crate::{ContinuityEvent, ContinuityIntent, ContinuityPlan, ContinuityRefusal, ContinuityTurn};
use sim_kernel::{Expr, Symbol};
use std::collections::BTreeSet;

/// Derived cache rebuilt solely from accepted turns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContinuityState {
    /// Accepted bounded history.
    pub turns: Vec<ContinuityTurn>,
    /// Whether cancellation has closed the session.
    pub cancelled: bool,
    /// Sequence expected for the next accepted event.
    pub next_sequence: u64,
    seen: BTreeSet<String>,
}

/// Applies one event without effects; duplicate events are idempotent.
pub fn apply(
    plan: &ContinuityPlan,
    state: &ContinuityState,
    event: ContinuityEvent,
) -> Result<ContinuityState, ContinuityRefusal> {
    plan.validate()
        .map_err(|e| refusal("invalid-plan", e.to_string()))?;
    if state.seen.contains(&event.event_id.to_string()) {
        return Ok(state.clone());
    }
    let next = state.next_sequence;
    if event.sequence != next {
        return Err(refusal(
            if event.sequence < next {
                "stale-event"
            } else {
                "reordered-event"
            },
            "event sequence is not next",
        ));
    }
    if state.cancelled {
        return Err(refusal("post-cancel-event", "session is cancelled"));
    }
    let role = plan
        .roles
        .iter()
        .find(|r| r.role == event.role)
        .ok_or_else(|| refusal("unknown-role", "role is not in plan"))?;
    if event
        .disclosure
        .as_ref()
        .is_some_and(|label| !plan.disclosure.contains(label))
    {
        return Err(refusal(
            "disclosure-policy",
            "payload disclosure is forbidden",
        ));
    }
    let mut intents = Vec::new();
    match event.kind.to_string().as_str() {
        "cancel" => intents.push(intent("cancelled", &event.role, None)),
        "candidate" => {
            let lease = event
                .lease
                .as_ref()
                .ok_or_else(|| refusal("missing-lease", "candidate requires route lease"))?;
            if lease.observed_at > lease.expires_at
                || lease.expires_at < event.logical_time
                || lease.observed_at > event.logical_time
                || event.logical_time - lease.observed_at > plan.max_freshness
            {
                return Err(refusal(
                    "stale-route",
                    "route lease is outside freshness bound",
                ));
            }
            if lease.networked
                && (matches!(plan.network, crate::NetworkPolicy::Offline)
                    || !plan.allowed_network_routes.contains(&lease.route))
            {
                return Err(refusal("network-policy", "network route is forbidden"));
            }
            if role
                .required_services
                .iter()
                .any(|s| !lease.services.contains(s))
            {
                return Err(refusal(
                    "service-closure",
                    "candidate lacks a required service",
                ));
            }
            intents.push(intent(
                "consider-route",
                &event.role,
                Some(lease.route.clone()),
            ));
        }
        _ => intents.push(intent("record", &event.role, None)),
    }
    let turn = ContinuityTurn {
        sequence: event.sequence,
        event_id: event.event_id.clone(),
        logical_time: event.logical_time,
        event: event.clone(),
        intents,
    };
    let mut out = state.clone();
    out.seen.insert(event.event_id.to_string());
    out.cancelled = event.kind.to_string() == "cancel";
    out.next_sequence += 1;
    out.turns.push(turn);
    let retain = usize::try_from(plan.retention_turns).unwrap_or(usize::MAX);
    if out.turns.len() > retain {
        out.turns.drain(..out.turns.len() - retain);
    }
    Ok(out)
}
/// Rebuilds derived state from an empty cache, rejecting any invalid journal history.
pub fn rebuild(
    plan: &ContinuityPlan,
    turns: &[ContinuityTurn],
) -> Result<ContinuityState, ContinuityRefusal> {
    let mut state = ContinuityState::default();
    for turn in turns {
        let next = apply(plan, &state, turn.event.clone())?;
        if next.turns.last() != Some(turn) {
            return Err(refusal(
                "journal-mismatch",
                "stored turn differs from pure reduction",
            ));
        }
        state = next;
    }
    Ok(state)
}
fn refusal(code: &str, detail: impl Into<String>) -> ContinuityRefusal {
    ContinuityRefusal {
        code: Symbol::new(code),
        detail: Expr::String(detail.into()),
    }
}
fn intent(kind: &str, role: &Symbol, route: Option<Symbol>) -> ContinuityIntent {
    ContinuityIntent {
        kind: Symbol::new(kind),
        role: role.clone(),
        route,
        detail: Expr::Nil,
    }
}
