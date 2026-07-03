use super::topology_helpers::{
    debate_state, judge_expr, judge_score, local_address, mesh_state, reply_state, ring_state,
    side_case,
};
use super::topology_runtime::{
    DebateSession, MeshSession, RingSession, evaluate_connection, is_stream_passthrough_frame,
    number_expr, reply_expr,
};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{EvalSite, ServerAddress, ServerFrame};
use std::{
    any::Any,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(super) struct RingTurnSite {
    pub(super) session: Arc<Mutex<RingSession>>,
    pub(super) agents: Vec<sim_lib_server::Connection>,
    pub(super) role_cycle: Vec<Symbol>,
}

#[derive(Clone)]
pub(super) struct MeshRoundSite {
    pub(super) session: Arc<Mutex<MeshSession>>,
    pub(super) agents: Vec<sim_lib_server::Connection>,
    pub(super) judge: Value,
}

#[derive(Clone)]
pub(super) struct DebateTurnSite {
    pub(super) session: Arc<Mutex<DebateSession>>,
    pub(super) pro: sim_lib_server::Connection,
    pub(super) con: sim_lib_server::Connection,
}

#[derive(Clone)]
pub(super) struct DebateJudgeSite {
    pub(super) session: Arc<Mutex<DebateSession>>,
    pub(super) judge: Value,
}

impl EvalSite for RingTurnSite {
    fn site_kind(&self) -> &'static str {
        "topology-ring-turn"
    }

    fn address(&self) -> &ServerAddress {
        local_address()
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if is_stream_passthrough_frame(cx, &frame)? {
            return Ok(frame);
        }
        let (current, agent_index, role_index, turns_used, max_turns, done) = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("ring session"))?;
            if session.turns_used == 0 && matches!(session.current, Expr::Nil) {
                session.current = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
            }
            (
                session.current.clone(),
                session.agent_index,
                session.role_index,
                session.turns_used,
                session.max_turns,
                session.done,
            )
        };
        if done {
            return reply_state(cx, &frame, ring_state(&self.session)?);
        }
        let agent = &self.agents[agent_index % self.agents.len()];
        let role = self
            .role_cycle
            .get(role_index % self.role_cycle.len().max(1))
            .cloned()
            .unwrap_or_else(|| Symbol::new("worker"));
        let reply = evaluate_connection(cx, agent, current, Some(role.clone()), &frame.envelope)?;
        let result = reply_expr(cx, &reply)?;
        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("ring session"))?;
            session.current = result.clone();
            session.turns_used = session.turns_used.saturating_add(1);
            session.agent_index = session.agent_index.saturating_add(1);
            session.role_index = session.role_index.saturating_add(1);
            session.transcript.push(Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("turn")),
                    number_expr(turns_used + 1),
                ),
                (
                    Expr::Symbol(Symbol::new("agent")),
                    Expr::String(agent.address().kind_symbol().to_string()),
                ),
                (Expr::Symbol(Symbol::new("role")), Expr::Symbol(role)),
                (Expr::Symbol(Symbol::new("value")), result),
            ]));
            session.done = session.turns_used >= max_turns;
        }
        reply_state(cx, &frame, ring_state(&self.session)?)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for MeshRoundSite {
    fn site_kind(&self) -> &'static str {
        "topology-mesh-round"
    }

    fn address(&self) -> &ServerAddress {
        local_address()
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if is_stream_passthrough_frame(cx, &frame)? {
            return Ok(frame);
        }
        let candidate = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("mesh session"))?;
            if session.rounds_used == 0 && matches!(session.candidate, Expr::Nil) {
                session.candidate = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
            }
            if session.done {
                return reply_state(cx, &frame, mesh_state(&self.session)?);
            }
            session.candidate.clone()
        };

        let mut scored = Vec::with_capacity(self.agents.len());
        for (index, agent) in self.agents.iter().enumerate() {
            let role = agent
                .role()
                .cloned()
                .unwrap_or_else(|| Symbol::new(if index % 2 == 0 { "worker" } else { "critic" }));
            let reply = evaluate_connection(
                cx,
                agent,
                candidate.clone(),
                Some(role.clone()),
                &frame.envelope,
            )?;
            let value = reply_expr(cx, &reply)?;
            let score = judge_score(cx, &self.judge, value.clone(), &frame.envelope)?;
            scored.push((role, value, score));
        }
        let best = scored
            .iter()
            .max_by(|left, right| {
                left.2
                    .partial_cmp(&right.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
            .ok_or_else(|| Error::Eval("mesh round had no candidates".to_owned()))?;
        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("mesh session"))?;
            let improved = session.best_score.is_none_or(|score| best.2 > score);
            session.rounds_used = session.rounds_used.saturating_add(1);
            let round = session.rounds_used;
            session.best_score = Some(best.2);
            session.candidate = best.1.clone();
            session.transcript.push(Expr::Map(vec![
                (Expr::Symbol(Symbol::new("round")), number_expr(round)),
                (
                    Expr::Symbol(Symbol::new("candidates")),
                    Expr::List(
                        scored
                            .iter()
                            .map(|(role, value, score)| {
                                Expr::Map(vec![
                                    (
                                        Expr::Symbol(Symbol::new("role")),
                                        Expr::Symbol(role.clone()),
                                    ),
                                    (Expr::Symbol(Symbol::new("value")), value.clone()),
                                    (Expr::Symbol(Symbol::new("score")), number_expr(*score)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (Expr::Symbol(Symbol::new("selected")), best.1.clone()),
                (Expr::Symbol(Symbol::new("score")), number_expr(best.2)),
            ]));
            session.done = session.rounds_used >= session.max_rounds || !improved;
        }
        reply_state(cx, &frame, mesh_state(&self.session)?)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for DebateTurnSite {
    fn site_kind(&self) -> &'static str {
        "topology-debate-turn"
    }

    fn address(&self) -> &ServerAddress {
        local_address()
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if is_stream_passthrough_frame(cx, &frame)? {
            return Ok(frame);
        }
        let (task, pro_turn, turns_used, done) = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("debate session"))?;
            if session.turns_used == 0 && matches!(session.task, Expr::Nil) {
                session.task = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
            }
            (
                session.task.clone(),
                session.pro_turn,
                session.turns_used,
                session.done,
            )
        };
        if done {
            return reply_state(cx, &frame, debate_state(&self.session)?);
        }
        let (side, role, connection) = if pro_turn {
            ("pro", Symbol::new("worker"), &self.pro)
        } else {
            ("con", Symbol::new("critic"), &self.con)
        };
        let context = Expr::Map(vec![
            (Expr::Symbol(Symbol::new("task")), task.clone()),
            (Expr::Symbol(Symbol::new("transcript")), {
                let session = self
                    .session
                    .lock()
                    .map_err(|_| Error::PoisonedLock("debate session"))?;
                Expr::List(session.transcript.clone())
            }),
        ]);
        let reply =
            evaluate_connection(cx, connection, context, Some(role.clone()), &frame.envelope)?;
        let contribution = reply_expr(cx, &reply)?;
        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("debate session"))?;
            session.turns_used = session.turns_used.saturating_add(1);
            session.pro_turn = !session.pro_turn;
            session.transcript.push(Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("turn")),
                    number_expr(turns_used + 1),
                ),
                (
                    Expr::Symbol(Symbol::new("side")),
                    Expr::String(side.to_owned()),
                ),
                (Expr::Symbol(Symbol::new("role")), Expr::Symbol(role)),
                (Expr::Symbol(Symbol::new("value")), contribution),
            ]));
            session.done = session.turns_used >= session.max_turns;
        }
        reply_state(cx, &frame, debate_state(&self.session)?)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for DebateJudgeSite {
    fn site_kind(&self) -> &'static str {
        "topology-debate-judge"
    }

    fn address(&self) -> &ServerAddress {
        local_address()
    }

    fn codecs(&self) -> &[Symbol] {
        &[]
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if is_stream_passthrough_frame(cx, &frame)? {
            return Ok(frame);
        }
        let transcript = {
            let session = self
                .session
                .lock()
                .map_err(|_| Error::PoisonedLock("debate session"))?;
            session.transcript.clone()
        };
        let pro_case = side_case(&transcript, "pro");
        let con_case = side_case(&transcript, "con");
        let pro_score = judge_score(cx, &self.judge, pro_case.clone(), &frame.envelope)?;
        let con_score = judge_score(cx, &self.judge, con_case.clone(), &frame.envelope)?;
        let verdict = judge_expr(
            cx,
            &self.judge,
            Expr::Map(vec![(
                Expr::Symbol(Symbol::new("transcript")),
                Expr::List(transcript.clone()),
            )]),
            &frame.envelope,
        )?;
        let winner = if pro_score >= con_score { "pro" } else { "con" };
        let result = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("winner")),
                Expr::String(winner.to_owned()),
            ),
            (Expr::Symbol(Symbol::new("verdict")), verdict),
            (
                Expr::Symbol(Symbol::new("transcript")),
                Expr::List(transcript),
            ),
        ]);
        super::topology_helpers::reply_expr_value(cx, &frame, result)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
