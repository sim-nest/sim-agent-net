use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use sim_kernel::{Cx, Error, EvalReply, EvalRequest, Result, Symbol, Value};
use sim_lib_server::{
    EvalSite, FrameEnvelope, ServerAddress, ServerFrame, eval_reply_from_frame,
    server_frame_from_reply, server_frame_from_request,
};

use crate::{memory::resolve_memory_backend, value_from_expr};

use super::swarm::{SwarmLoopSession, SwarmMember, SwarmRoundRecord};
use super::swarm_support::{next_role, reply_cost, session_status, state_expr};
use super::types::SwarmRegistry;

#[derive(Clone)]
pub(super) struct SwarmPlannerSite {
    pub(super) session: Arc<Mutex<SwarmLoopSession>>,
}

#[derive(Clone)]
pub(super) struct SwarmMemberSite {
    pub(super) session: Arc<Mutex<SwarmLoopSession>>,
    pub(super) member: SwarmMember,
}

#[derive(Clone)]
pub(super) struct SwarmFinalizeSite {
    pub(super) session: Arc<Mutex<SwarmLoopSession>>,
    pub(super) blackboard: Option<Value>,
    pub(super) registry: Arc<Mutex<SwarmRegistry>>,
}

impl EvalSite for SwarmPlannerSite {
    fn site_kind(&self) -> &'static str {
        "swarm-planner"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: std::sync::OnceLock<ServerAddress> = std::sync::OnceLock::new();
        LOCAL.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let consistency = frame.envelope.consistency;
        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
            if !session.done && !session.budget_exhausted {
                session.planner.pending_role =
                    next_role(&session.planner.available_roles, &session.last_round);
            }
        }
        let value = value_from_expr(cx, &state_expr(&self.session)?)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for SwarmMemberSite {
    fn site_kind(&self) -> &'static str {
        "swarm-member"
    }

    fn address(&self) -> &ServerAddress {
        self.member.connection.address()
    }

    fn codecs(&self) -> &[Symbol] {
        self.member.connection.supported_codecs()
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let consistency = frame.envelope.consistency;
        let (should_run, turn, current) = {
            let session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
            (
                session.planner.pending_role.as_ref() == Some(&self.member.role),
                session.turns_used.saturating_add(1),
                session.current.clone(),
            )
        };
        if !should_run {
            let value = value_from_expr(cx, &state_expr(&self.session)?)?;
            let diagnostics = cx.take_diagnostics();
            return server_frame_from_reply(
                cx,
                &frame.codec,
                EvalReply {
                    value,
                    diagnostics,
                    trace: None,
                },
                consistency,
            );
        }

        let request = EvalRequest {
            expr: current,
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: frame.envelope.required_capabilities.clone(),
            deadline: frame.envelope.deadline,
            consistency,
            trace: frame.envelope.trace,
        };
        let mut member_frame =
            server_frame_from_request(cx, self.member.connection.default_codec(), request)?;
        member_frame.envelope = FrameEnvelope {
            role: Some(self.member.role.clone()),
            hop: frame.envelope.hop.saturating_add(1),
            ..member_frame.envelope
        };
        let reply = self.member.connection.site().answer(cx, member_frame)?;
        let reply_expr = eval_reply_from_frame(cx, &reply)?
            .value
            .object()
            .as_expr(cx)?;
        let cost = reply_cost(&reply_expr);

        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
            session.current = reply_expr.clone();
            session.last_value = reply_expr.clone();
            session.cost_used += cost;
            session.round_records.push(SwarmRoundRecord {
                turn,
                role: reply
                    .envelope
                    .role
                    .clone()
                    .unwrap_or_else(|| self.member.role.clone()),
                value: reply_expr,
            });
        }

        let value = value_from_expr(cx, &state_expr(&self.session)?)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for SwarmFinalizeSite {
    fn site_kind(&self) -> &'static str {
        "swarm-finalize"
    }

    fn address(&self) -> &ServerAddress {
        static LOCAL: std::sync::OnceLock<ServerAddress> = std::sync::OnceLock::new();
        LOCAL.get_or_init(|| ServerAddress::Local)
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let consistency = frame.envelope.consistency;
        let records = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("swarm loop session"))?;
            if !session.done && !session.budget_exhausted && session.planner.pending_role.is_some()
            {
                session.turns_used = session.turns_used.saturating_add(1);
                let round_records = session.round_records.clone();
                session.transcript.extend(round_records.clone());
                session.last_round = round_records;
                session.round_records.clear();
                session.planner.pending_role = None;
                if session.turns_used >= session.max_turns {
                    session.done = true;
                }
                if session
                    .max_cost
                    .is_some_and(|limit| session.cost_used >= limit)
                {
                    session.done = true;
                    session.budget_exhausted = true;
                }
            }
            session.last_round.clone()
        };

        if let Some(blackboard) = &self.blackboard {
            let memory = resolve_memory_backend(blackboard)?;
            for record in &records {
                let value = value_from_expr(cx, &record.expr())?;
                memory.append(cx, value)?;
            }
        }

        {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| Error::PoisonedLock("swarm registry"))?;
            registry.status = session_status(&self.session)?;
        }

        let value = value_from_expr(cx, &state_expr(&self.session)?)?;
        let diagnostics = cx.take_diagnostics();
        server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value,
                diagnostics,
                trace: None,
            },
            consistency,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
