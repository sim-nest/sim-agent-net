use super::lookup::connection_from_value;
use super::swarm_sites::{SwarmFinalizeSite, SwarmMemberSite, SwarmPlannerSite};
use super::swarm_support::{
    default_member_role, first_codec_for_swarm, number_expr, planner_strategy, session_status,
    session_transcript_expr, state_expr, until_value, update_registry_status,
};
use super::types::{AgentFabric, SwarmRunRecord};
use crate::agents::ops::shared::agent_connection_for_value;
use crate::{AgentRole, expr_to_value, installed_codecs};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{
    Connection, EvalSite, LoopEvalSite, PipelineEvalSite, ServerAddress, eval_reply_from_frame,
};
use std::sync::{Arc, Mutex};

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
pub(super) struct SwarmLoopSession {
    pub(super) task_id: String,
    pub(super) current: Expr,
    pub(super) planner: SwarmPlannerState,
    pub(super) max_turns: u32,
    pub(super) max_cost: Option<f64>,
    pub(super) transcript: Vec<SwarmRoundRecord>,
    pub(super) last_round: Vec<SwarmRoundRecord>,
    pub(super) round_records: Vec<SwarmRoundRecord>,
    pub(super) turns_used: u32,
    pub(super) cost_used: f64,
    pub(super) budget_exhausted: bool,
    pub(super) done: bool,
    pub(super) active: bool,
    pub(super) last_value: Expr,
}

#[derive(Clone)]
pub(super) struct SwarmPlannerState {
    pub(super) strategy: Symbol,
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
    let session = Arc::new(Mutex::new(SwarmLoopSession {
        task_id: task_id.clone(),
        current: expr.clone(),
        planner: SwarmPlannerState {
            strategy: planner_strategy(fabric.planner.as_ref()),
            available_roles,
            pending_role: None,
        },
        max_turns,
        max_cost: fabric.budget.as_ref().and_then(|budget| budget.max_cost),
        transcript: Vec::new(),
        last_round: Vec::new(),
        round_records: Vec::new(),
        turns_used: 0,
        cost_used: 0.0,
        budget_exhausted: false,
        done: false,
        active: true,
        last_value: expr,
    }));
    update_registry_status(&fabric.runs, &session)?;

    let mut steps = Vec::with_capacity(members.len() + 2);
    steps.push(Connection::new(
        ServerAddress::Local,
        first_codec_for_swarm(cx),
        installed_codecs(cx),
        Arc::new(SwarmPlannerSite {
            session: session.clone(),
        }),
    )?);
    for member in members {
        let role = member.role.clone();
        steps.push(Connection::with_session(
            ServerAddress::Local,
            member.connection.default_codec().clone(),
            member.connection.supported_codecs().to_vec(),
            Arc::new(SwarmMemberSite {
                session: session.clone(),
                member,
            }),
            Some(role),
            sim_lib_server::IsolationPolicy::default(),
        )?);
    }
    steps.push(Connection::new(
        ServerAddress::Local,
        first_codec_for_swarm(cx),
        installed_codecs(cx),
        Arc::new(SwarmFinalizeSite {
            session: session.clone(),
            blackboard: fabric.blackboard.clone(),
            registry: fabric.runs.clone(),
        }),
    )?);

    let loop_site = LoopEvalSite::new(
        ServerAddress::Pipeline {
            steps: steps.iter().map(|step| step.address().clone()).collect(),
        },
        installed_codecs(cx),
        steps,
        usize::try_from(max_turns).unwrap_or(usize::MAX),
        until_value(cx)?,
    );
    let codec = first_codec_for_swarm(cx);
    let request = sim_lib_server::server_frame_from_request(
        cx,
        &codec,
        sim_kernel::EvalRequest {
            expr: state_expr(&session)?,
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
    )?;
    let reply = loop_site.answer(cx, request)?;
    let result = eval_reply_from_frame(cx, &reply)?
        .value
        .object()
        .as_expr(cx)?;

    let mut registry = fabric
        .runs
        .lock()
        .map_err(|_| Error::PoisonedLock("swarm registry"))?;
    let transcript = session_transcript_expr(&session)?;
    registry.last_task_id = Some(task_id.clone());
    registry.last_run = Some(result.clone());
    registry.history.insert(
        task_id.clone(),
        SwarmRunRecord {
            transcript: transcript.clone(),
        },
    );
    registry.status = session_status(&session)?;
    registry.status.active = false;
    drop(registry);

    {
        let mut state = session
            .lock()
            .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
        state.active = false;
    }

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
    let last_value = expr_to_value(cx, &status.last_value)?;
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

fn swarm_members(_cx: &mut Cx, fabric: &AgentFabric) -> Result<Vec<SwarmMember>> {
    if let Some(topology) = &fabric.topology
        && let Some(connection) = connection_from_value(topology)
    {
        return topology_members(connection).or_else(|_| {
            Ok(vec![SwarmMember {
                role: connection
                    .role()
                    .cloned()
                    .unwrap_or_else(|| AgentRole::Worker.as_symbol()),
                connection: connection.clone(),
            }])
        });
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

fn topology_members(connection: &Connection) -> Result<Vec<SwarmMember>> {
    let Some(pipeline) = connection
        .site()
        .as_any()
        .downcast_ref::<PipelineEvalSite>()
    else {
        return Err(Error::TypeMismatch {
            expected: "pipeline topology",
            found: "non-pipeline topology",
        });
    };
    pipeline
        .steps()
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let role = step
                .role()
                .cloned()
                .unwrap_or_else(|| default_member_role(index));
            let connection = Connection::with_session(
                step.address().clone(),
                step.default_codec().clone(),
                step.supported_codecs().to_vec(),
                step.site().clone(),
                Some(role.clone()),
                step.session().isolation.clone(),
            )?;
            Ok(SwarmMember { connection, role })
        })
        .collect()
}
