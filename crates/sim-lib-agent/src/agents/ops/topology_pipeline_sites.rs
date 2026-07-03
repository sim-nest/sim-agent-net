use super::topology_helpers::{local_address, reply_expr_value, select_market_winner};
use super::topology_runtime::{
    evaluate_connection, expr_field, is_stream_passthrough_frame, reply_expr,
};
use sim_kernel::{Cx, Expr, Result, Symbol, Value};
use sim_lib_server::{EvalSite, ServerAddress, ServerFrame};
use std::any::Any;

#[derive(Clone)]
pub(super) struct StarSite {
    pub(super) hub: sim_lib_server::Connection,
    pub(super) spokes: Vec<sim_lib_server::Connection>,
    pub(super) hub_role: Symbol,
    pub(super) spoke_role: Symbol,
}

#[derive(Clone)]
pub(super) struct MarketRouteSite {
    pub(super) workers: Vec<Value>,
    pub(super) router: Value,
}

#[derive(Clone)]
pub(super) struct SpeculateSite {
    pub(super) speculator: sim_lib_server::Connection,
}

#[derive(Clone)]
pub(super) struct VerifySite {
    pub(super) verifier: sim_lib_server::Connection,
    pub(super) on_mismatch: Symbol,
}

impl EvalSite for StarSite {
    fn site_kind(&self) -> &'static str {
        "topology-star"
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
        let task = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let hub_reply = evaluate_connection(
            cx,
            &self.hub,
            task.clone(),
            Some(self.hub_role.clone()),
            &frame.envelope,
        )?;
        let hub_input = reply_expr(cx, &hub_reply)?;
        let mut spoke_replies = Vec::with_capacity(self.spokes.len());
        for spoke in &self.spokes {
            let reply = evaluate_connection(
                cx,
                spoke,
                hub_input.clone(),
                Some(self.spoke_role.clone()),
                &frame.envelope,
            )?;
            spoke_replies.push(reply_expr(cx, &reply)?);
        }
        let merge_input = Expr::Map(vec![
            (Expr::Symbol(Symbol::new("task")), task),
            (Expr::Symbol(Symbol::new("hub-input")), hub_input.clone()),
            (
                Expr::Symbol(Symbol::new("spoke-replies")),
                Expr::List(spoke_replies.clone()),
            ),
        ]);
        let merged = evaluate_connection(
            cx,
            &self.hub,
            merge_input,
            Some(self.hub_role.clone()),
            &frame.envelope,
        )?;
        let result = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("result")),
                reply_expr(cx, &merged)?,
            ),
            (Expr::Symbol(Symbol::new("hub-input")), hub_input),
            (
                Expr::Symbol(Symbol::new("spoke-replies")),
                Expr::List(spoke_replies),
            ),
        ]);
        reply_expr_value(cx, &frame, result)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for MarketRouteSite {
    fn site_kind(&self) -> &'static str {
        "topology-market"
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
        let task = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let (winner, bid) =
            select_market_winner(cx, &self.workers, &self.router, &task, &frame.envelope)?;
        let worker = super::shared::agent_connection_for_value(winner.clone())?;
        let reply = evaluate_connection(
            cx,
            &worker,
            task,
            worker
                .role()
                .cloned()
                .or_else(|| Some(Symbol::new("worker"))),
            &frame.envelope,
        )?;
        let result = Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("winner")),
                Expr::String(super::topology_runtime::text_label(&winner)),
            ),
            (
                Expr::Symbol(Symbol::new("bid")),
                super::topology_runtime::number_expr(bid),
            ),
            (Expr::Symbol(Symbol::new("result")), reply_expr(cx, &reply)?),
        ]);
        reply_expr_value(cx, &frame, result)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for SpeculateSite {
    fn site_kind(&self) -> &'static str {
        "topology-speculate"
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
        let task = sim_lib_server::eval_request_from_frame(cx, &frame)?.expr;
        let reply = evaluate_connection(
            cx,
            &self.speculator,
            task.clone(),
            Some(Symbol::new("worker")),
            &frame.envelope,
        )?;
        let state = Expr::Map(vec![
            (Expr::Symbol(Symbol::new("task")), task),
            (
                Expr::Symbol(Symbol::new("speculative")),
                reply_expr(cx, &reply)?,
            ),
            (Expr::Symbol(Symbol::new("done")), Expr::Bool(false)),
        ]);
        reply_expr_value(cx, &frame, state)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl EvalSite for VerifySite {
    fn site_kind(&self) -> &'static str {
        "topology-verify"
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
        let task = expr_field(&state, "task")?;
        let speculative = expr_field(&state, "speculative")?;
        let verify_input = Expr::Map(vec![
            (Expr::Symbol(Symbol::new("task")), task.clone()),
            (Expr::Symbol(Symbol::new("answer")), speculative.clone()),
        ]);
        let verified = evaluate_connection(
            cx,
            &self.verifier,
            verify_input,
            Some(Symbol::new("verifier")),
            &frame.envelope,
        )?;
        let verified_expr = reply_expr(cx, &verified)?;
        let agreed = verified_expr == speculative;
        let final_result = if agreed {
            speculative.clone()
        } else if self.on_mismatch.name.as_ref() == "escalate" {
            verified_expr.clone()
        } else {
            let retry = evaluate_connection(
                cx,
                &self.verifier,
                task,
                Some(Symbol::new("verifier")),
                &frame.envelope,
            )?;
            reply_expr(cx, &retry)?
        };
        let result = Expr::Map(vec![
            (Expr::Symbol(Symbol::new("done")), Expr::Bool(true)),
            (Expr::Symbol(Symbol::new("agreed")), Expr::Bool(agreed)),
            (Expr::Symbol(Symbol::new("mismatch")), Expr::Bool(!agreed)),
            (
                Expr::Symbol(Symbol::new("policy")),
                Expr::Symbol(self.on_mismatch.clone()),
            ),
            (Expr::Symbol(Symbol::new("result")), final_result),
            (Expr::Symbol(Symbol::new("speculative")), speculative),
            (Expr::Symbol(Symbol::new("verifier")), verified_expr),
        ]);
        reply_expr_value(cx, &frame, result)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
