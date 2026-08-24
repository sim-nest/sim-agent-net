use super::topology_helpers::{judge_expr, judge_score, local_address, reply_state, side_case};
use super::topology_runtime::{
    evaluate_connection, is_stream_passthrough_frame, map_field, number_expr, reply_expr,
};
use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::{EvalSite, ServerAddress, ServerFrame};
use std::any::Any;

#[derive(Clone)]
pub(super) struct RingTurnSite {
    pub(super) agents: Vec<sim_lib_server::Connection>,
    pub(super) role_cycle: Vec<Symbol>,
    pub(super) max_turns: u32,
}

#[derive(Clone)]
pub(super) struct MeshRoundSite {
    pub(super) agents: Vec<sim_lib_server::Connection>,
    pub(super) judge: Value,
    pub(super) max_rounds: u32,
}

#[derive(Clone)]
pub(super) struct DebateTurnSite {
    pub(super) pro: sim_lib_server::Connection,
    pub(super) con: sim_lib_server::Connection,
    pub(super) max_turns: u32,
}

#[derive(Clone)]
pub(super) struct DebateJudgeSite {
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
        let input = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let turns_used = u32_field(&input, "turns-used").unwrap_or(0);
        let mut transcript = list_field(&input, "transcript");
        let current = map_field(&input, "result").cloned().unwrap_or(input);
        // A state frame owns every scheduling cursor. No site-local session is retained.
        let agent_index = usize::try_from(turns_used).unwrap_or(usize::MAX);
        let role_index = agent_index;
        let agent = &self.agents[agent_index % self.agents.len()];
        let role = self
            .role_cycle
            .get(role_index % self.role_cycle.len().max(1))
            .cloned()
            .unwrap_or_else(|| Symbol::new("worker"));
        let reply = evaluate_connection(cx, agent, current, Some(role.clone()), &frame.envelope)?;
        let result = reply_expr(cx, &reply)?;
        let next_turn = turns_used.saturating_add(1);
        transcript.push(Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("turn")),
                number_expr(turns_used + 1),
            ),
            (
                Expr::Symbol(Symbol::new("agent")),
                Expr::String(agent.address().kind_symbol().to_string()),
            ),
            (Expr::Symbol(Symbol::new("role")), Expr::Symbol(role)),
            (Expr::Symbol(Symbol::new("value")), result.clone()),
        ]));
        reply_state(
            cx,
            &frame,
            Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("done")),
                    Expr::Bool(next_turn >= self.max_turns),
                ),
                (Expr::Symbol(Symbol::new("result")), result),
                (
                    Expr::Symbol(Symbol::new("transcript")),
                    Expr::List(transcript),
                ),
                (
                    Expr::Symbol(Symbol::new("turns-used")),
                    number_expr(next_turn),
                ),
            ]),
        )
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
        let input = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let rounds_used = u32_field(&input, "rounds-used").unwrap_or(0);
        let previous_score = float_field(&input, "score");
        let mut transcript = list_field(&input, "transcript");
        let candidate = map_field(&input, "candidate").cloned().unwrap_or(input);

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
        let improved = previous_score.is_none_or(|score| best.2 > score);
        let round = rounds_used.saturating_add(1);
        transcript.push(Expr::Map(vec![
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
        reply_state(
            cx,
            &frame,
            Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("done")),
                    Expr::Bool(round >= self.max_rounds || !improved),
                ),
                (Expr::Symbol(Symbol::new("candidate")), best.1),
                (Expr::Symbol(Symbol::new("score")), number_expr(best.2)),
                (
                    Expr::Symbol(Symbol::new("transcript")),
                    Expr::List(transcript),
                ),
                (Expr::Symbol(Symbol::new("rounds-used")), number_expr(round)),
            ]),
        )
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
        let input = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let turns_used = u32_field(&input, "turns-used").unwrap_or(0);
        let task = map_field(&input, "task")
            .cloned()
            .unwrap_or_else(|| input.clone());
        let mut transcript = list_field(&input, "transcript");
        let pro_turn = turns_used % 2 == 0;
        let (side, role, connection) = if pro_turn {
            ("pro", Symbol::new("worker"), &self.pro)
        } else {
            ("con", Symbol::new("critic"), &self.con)
        };
        let context = Expr::Map(vec![
            (Expr::Symbol(Symbol::new("task")), task.clone()),
            (
                Expr::Symbol(Symbol::new("transcript")),
                Expr::List(transcript.clone()),
            ),
        ]);
        let reply =
            evaluate_connection(cx, connection, context, Some(role.clone()), &frame.envelope)?;
        let contribution = reply_expr(cx, &reply)?;
        let next_turn = turns_used.saturating_add(1);
        transcript.push(Expr::Map(vec![
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
        reply_state(
            cx,
            &frame,
            Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("done")),
                    Expr::Bool(next_turn >= self.max_turns),
                ),
                (Expr::Symbol(Symbol::new("task")), task),
                (
                    Expr::Symbol(Symbol::new("transcript")),
                    Expr::List(transcript),
                ),
                (
                    Expr::Symbol(Symbol::new("turns-used")),
                    number_expr(next_turn),
                ),
            ]),
        )
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
        let state = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let transcript = list_field(&state, "transcript");
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

fn list_field(expr: &Expr, name: &str) -> Vec<Expr> {
    match map_field(expr, name) {
        Some(Expr::List(values)) => values.clone(),
        _ => Vec::new(),
    }
}

fn float_field(expr: &Expr, name: &str) -> Option<f64> {
    match map_field(expr, name) {
        Some(Expr::Number(number)) => number.canonical.parse().ok(),
        _ => None,
    }
}

fn u32_field(expr: &Expr, name: &str) -> Option<u32> {
    float_field(expr, name).and_then(|value| u32::try_from(value as u64).ok())
}
