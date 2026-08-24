use super::lookup::connection_from_value;
use super::swarm_support::{default_member_role, next_role, reply_cost};
use super::types::{AgentFabric, SwarmRunRecord};
use crate::agents::ops::shared::agent_connection_for_value;
use crate::{AgentRole, value_from_expr};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{Connection, eval_reply_from_frame, server_frame_from_request};

#[derive(Clone)]
pub(super) struct SwarmMember {
    pub(super) connection: Connection,
    pub(super) role: Symbol,
}

#[derive(Clone)]
pub(super) struct SwarmRoundRecord {
    pub(super) turn: u32,
    pub(super) role: Symbol,
    pub(super) value: Expr,
}

#[derive(Clone)]
pub(super) struct SwarmRunState {
    pub(super) task_id: String,
    pub(super) current: Expr,
    pub(super) planner: SwarmPlannerState,
    pub(super) max_turns: u32,
    pub(super) max_cost: Option<f64>,
    pub(super) transcript: Vec<SwarmRoundRecord>,
    pub(super) last_round: Vec<SwarmRoundRecord>,
    pub(super) turns_used: u32,
    pub(super) cost_used: f64,
    pub(super) budget_exhausted: bool,
    pub(super) done: bool,
    pub(super) active: bool,
    pub(super) last_value: Expr,
}

#[derive(Clone)]
pub(super) struct SwarmPlannerState {
    pub(super) available_roles: Vec<Symbol>,
    pub(super) pending_role: Option<Symbol>,
}

pub(crate) fn next_swarm_task_id(fabric: &AgentFabric) -> Result<String> {
    let mut registry = fabric
        .runs
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm registry"))?;
    let task_id = format!("{}-{}", fabric.name, registry.next_task_id);
    registry.next_task_id = registry.next_task_id.saturating_add(1);
    registry.status.active = true;
    registry.status.task_id = Some(task_id.clone());
    Ok(task_id)
}

pub(crate) fn swarm_realize_expr(cx: &mut Cx, fabric: &AgentFabric, expr: Expr) -> Result<Expr> {
    let task_id = next_swarm_task_id(fabric)?;
    let members = swarm_members(cx, fabric)?;
    let available_roles = members
        .iter()
        .map(|member| member.role.clone())
        .collect::<Vec<_>>();
    let max_turns = fabric
        .budget
        .as_ref()
        .and_then(|budget| budget.max_turns)
        .unwrap_or_else(|| u32::try_from(available_roles.len().max(1)).unwrap_or(u32::MAX));
    let mut state = SwarmRunState {
        task_id: task_id.clone(),
        current: expr.clone(),
        planner: SwarmPlannerState {
            available_roles,
            pending_role: None,
        },
        max_turns,
        max_cost: fabric.budget.as_ref().and_then(|budget| budget.max_cost),
        transcript: Vec::new(),
        last_round: Vec::new(),
        turns_used: 0,
        cost_used: 0.0,
        budget_exhausted: false,
        done: false,
        active: true,
        last_value: expr,
    };

    while !state.done {
        state.planner.pending_role = next_role(&state.planner.available_roles, &state.last_round);
        let Some(role) = state.planner.pending_role.clone() else {
            state.done = true;
            break;
        };
        let member = members
            .iter()
            .find(|member| member.role == role)
            .ok_or_else(|| Error::Eval(format!("swarm role {role} has no bound member")))?;
        let request = sim_kernel::EvalRequest {
            expr: state.current.clone(),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: sim_kernel::Consistency::LocalFirst,
            trace: false,
        };
        let mut member_frame =
            server_frame_from_request(cx, member.connection.default_codec(), request)?;
        member_frame.envelope.role = Some(role.clone());
        let reply = member.connection.site().answer(cx, member_frame)?;
        let value = eval_reply_from_frame(cx, &reply)?
            .value
            .object()
            .as_expr(cx)?;
        state.turns_used = state.turns_used.saturating_add(1);
        state.cost_used += reply_cost(&value);
        let record = SwarmRoundRecord {
            turn: state.turns_used,
            role: reply.envelope.role.clone().unwrap_or(role),
            value: value.clone(),
        };
        state.current = value.clone();
        state.last_value = value;
        state.last_round = vec![record.clone()];
        state.transcript.push(record.clone());
        if let Some(blackboard) = &fabric.blackboard {
            let memory = crate::memory::resolve_memory_backend(blackboard)?;
            let record_value = value_from_expr(cx, &record.expr())?;
            memory.append(cx, record_value)?;
        }
        state.budget_exhausted = state.max_cost.is_some_and(|limit| state.cost_used >= limit);
        state.done = state.turns_used >= state.max_turns || state.budget_exhausted;
    }
    state.active = false;
    let result = state.last_value.clone();

    let mut registry = fabric
        .runs
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm registry"))?;
    let transcript = Expr::List(
        state
            .transcript
            .iter()
            .map(SwarmRoundRecord::expr)
            .collect(),
    );
    registry.last_task_id = Some(task_id.clone());
    registry.last_run = Some(result.clone());
    registry.history.insert(
        task_id.clone(),
        SwarmRunRecord {
            transcript: transcript.clone(),
        },
    );
    registry.status = state.status();

    Ok(result)
}

pub(crate) fn swarm_explain_expr(fabric: &AgentFabric, task_id: Option<&str>) -> Result<Expr> {
    let registry = fabric
        .runs
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm registry"))?;
    let selected = task_id
        .map(str::to_owned)
        .or_else(|| registry.last_task_id.clone());
    let Some(task_id) = selected else {
        return Ok(Expr::Nil);
    };
    Ok(registry
        .history
        .get(&task_id)
        .map(|record| record.transcript.clone())
        .unwrap_or(Expr::Nil))
}

pub(crate) fn swarm_status_value_for_table(cx: &mut Cx, fabric: &AgentFabric) -> Result<Value> {
    let status = fabric
        .runs
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm registry"))?
        .status
        .clone();
    let last_value = value_from_expr(cx, &status.last_value)?;
    cx.factory().table(vec![
        (Symbol::new("active"), cx.factory().bool(status.active)?),
        (
            Symbol::new("task-id"),
            match &status.task_id {
                Some(task_id) => cx.factory().string(task_id.clone())?,
                None => cx.factory().nil()?,
            },
        ),
        (
            Symbol::new("turns-used"),
            cx.factory().string(status.turns_used.to_string())?,
        ),
        (
            Symbol::new("turns-remaining"),
            match status.turns_remaining {
                Some(remaining) => cx.factory().string(remaining.to_string())?,
                None => cx.factory().nil()?,
            },
        ),
        (
            Symbol::new("cost-used"),
            cx.factory().string(status.cost_used.to_string())?,
        ),
        (
            Symbol::new("cost-remaining"),
            match status.cost_remaining {
                Some(remaining) => cx.factory().string(remaining.to_string())?,
                None => cx.factory().nil()?,
            },
        ),
        (
            Symbol::new("budget-exhausted"),
            cx.factory().bool(status.budget_exhausted)?,
        ),
        (Symbol::new("last-value"), last_value),
    ])
}

impl SwarmRoundRecord {
    pub(super) fn expr(&self) -> Expr {
        Expr::Map(vec![
            (Expr::Symbol(Symbol::new("turn")), number_expr(self.turn)),
            (
                Expr::Symbol(Symbol::new("role")),
                Expr::Symbol(self.role.clone()),
            ),
            (Expr::Symbol(Symbol::new("value")), self.value.clone()),
        ])
    }
}

impl SwarmRunState {
    fn status(&self) -> super::types::SwarmStatus {
        super::types::SwarmStatus {
            active: self.active,
            task_id: Some(self.task_id.clone()),
            turns_used: self.turns_used,
            turns_remaining: Some(self.max_turns.saturating_sub(self.turns_used)),
            cost_used: self.cost_used,
            cost_remaining: self.max_cost.map(|limit| (limit - self.cost_used).max(0.0)),
            budget_exhausted: self.budget_exhausted,
            last_value: self.last_value.clone(),
        }
    }
}

fn swarm_members(_cx: &mut Cx, fabric: &AgentFabric) -> Result<Vec<SwarmMember>> {
    if let Some(topology) = &fabric.topology
        && let Some(connection) = connection_from_value(topology)
    {
        return Ok(vec![SwarmMember {
            role: connection
                .role()
                .cloned()
                .unwrap_or_else(|| AgentRole::Worker.as_symbol()),
            connection: connection.clone(),
        }]);
    }

    fabric
        .members
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let base = agent_connection_for_value(value.clone())?;
            let role = base
                .role()
                .cloned()
                .unwrap_or_else(|| default_member_role(index));
            let connection = Connection::with_session(
                base.address().clone(),
                base.default_codec().clone(),
                base.supported_codecs().to_vec(),
                base.site().clone(),
                Some(role.clone()),
                base.session().isolation.clone(),
            )?;
            Ok(SwarmMember { connection, role })
        })
        .collect()
}
