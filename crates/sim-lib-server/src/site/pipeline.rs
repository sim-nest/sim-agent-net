use sim_kernel::{
    Args, Cx, Error, EvalMode, EvalRequest, Expr, ReadPolicy, Result, Symbol, Value,
    read_eval_capability,
};
use sim_lib_stream_core::{ClockDomain, LatencyClass, StreamEndpoint, StreamEndpointKind};

use crate::{
    Connection, FrameKind, ServerAddress, ServerFrame, eval_reply_from_frame,
    eval_request_from_frame, server_frame_from_reply, server_frame_from_request,
};

use super::core::{
    EvalSite, Site, SiteKind, eval_site_clock_domain, eval_site_endpoint_kind,
    eval_site_latency_class, reply_codec_for_frame, site_endpoint_id,
};

/// An eval site that threads each request through an ordered chain of
/// connection steps, feeding each step's reply into the next.
#[derive(Clone)]
pub struct PipelineEvalSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
    steps: Vec<Connection>,
}

impl PipelineEvalSite {
    /// Creates a pipeline site answering at `address` over `codecs`, running the
    /// given ordered `steps`.
    pub fn new(address: ServerAddress, codecs: Vec<Symbol>, steps: Vec<Connection>) -> Self {
        Self {
            address,
            codecs,
            steps,
        }
    }

    /// Returns the pipeline's ordered connection steps.
    pub fn steps(&self) -> &[Connection] {
        &self.steps
    }
}

impl EvalSite for PipelineEvalSite {
    fn site_kind(&self) -> &'static str {
        "pipeline"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if self.steps.is_empty() {
            return Err(Error::Eval(
                "pipeline eval site requires at least one step".to_owned(),
            ));
        }

        match frame.kind {
            FrameKind::Request => answer_pipeline_request(cx, self, &self.steps, frame),
            _ => answer_pipeline_passthrough(cx, &self.steps, frame),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StreamEndpoint for PipelineEvalSite {
    fn endpoint_id(&self) -> Symbol {
        site_endpoint_id(SiteKind::Pipeline, &self.address)
    }

    fn endpoint_kind(&self) -> StreamEndpointKind {
        eval_site_endpoint_kind()
    }

    fn clock_domain(&self) -> ClockDomain {
        eval_site_clock_domain()
    }

    fn latency_class(&self) -> LatencyClass {
        eval_site_latency_class()
    }
}

impl Site for PipelineEvalSite {
    fn kind(&self) -> SiteKind {
        SiteKind::Pipeline
    }
}

/// An eval site that repeatedly runs its pipeline steps -- up to
/// `max_iterations` times -- until the `until` predicate fires on a reply.
#[derive(Clone)]
pub struct LoopEvalSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
    steps: Vec<Connection>,
    max_iterations: usize,
    until: Value,
}

impl LoopEvalSite {
    /// Creates a loop site answering at `address` over `codecs`, running `steps`
    /// at most `max_iterations` times or until `until` fires.
    pub fn new(
        address: ServerAddress,
        codecs: Vec<Symbol>,
        steps: Vec<Connection>,
        max_iterations: usize,
        until: Value,
    ) -> Self {
        Self {
            address,
            codecs,
            steps,
            max_iterations,
            until,
        }
    }
}

impl EvalSite for LoopEvalSite {
    fn site_kind(&self) -> &'static str {
        "loop"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if self.steps.is_empty() {
            return Err(Error::Eval(
                "loop eval site requires at least one step".to_owned(),
            ));
        }

        let mut current = frame;
        for _ in 0..self.max_iterations {
            current = match current.kind {
                FrameKind::Request => answer_pipeline_request(cx, self, &self.steps, current)?,
                FrameKind::Response => {
                    let request_frame = response_as_request(cx, current)?;
                    answer_pipeline_request(cx, self, &self.steps, request_frame)?
                }
                _ => answer_pipeline_passthrough(cx, &self.steps, current)?,
            };
            if loop_until_fired(cx, &self.until, &current)? {
                return Ok(current);
            }
        }
        Ok(current)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StreamEndpoint for LoopEvalSite {
    fn endpoint_id(&self) -> Symbol {
        site_endpoint_id(SiteKind::Loop, &self.address)
    }

    fn endpoint_kind(&self) -> StreamEndpointKind {
        eval_site_endpoint_kind()
    }

    fn clock_domain(&self) -> ClockDomain {
        eval_site_clock_domain()
    }

    fn latency_class(&self) -> LatencyClass {
        eval_site_latency_class()
    }
}

impl Site for LoopEvalSite {
    fn kind(&self) -> SiteKind {
        SiteKind::Loop
    }
}

fn answer_pipeline_request(
    cx: &mut Cx,
    site: &dyn EvalSite,
    steps: &[Connection],
    frame: ServerFrame,
) -> Result<ServerFrame> {
    let request = eval_request_from_frame(cx, &frame)?;
    let mut current_expr = request.expr;
    let mut last_reply = None;
    let consistency = frame.envelope.consistency;

    for (index, step) in steps.iter().enumerate() {
        let step_request = EvalRequest {
            expr: current_expr,
            result_shape: None,
            required_capabilities: frame.envelope.required_capabilities.clone(),
            deadline: frame.envelope.deadline,
            consistency,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: frame.envelope.trace,
        };
        let mut step_frame = server_frame_from_request(cx, step.default_codec(), step_request)?;
        step_frame.msg_id = frame.msg_id;
        step_frame.correlate = frame.correlate;
        step_frame.envelope.reply_codec_hint = frame.envelope.reply_codec_hint.clone();
        step_frame.envelope.role = step.role().cloned().or_else(|| frame.envelope.role.clone());
        step_frame.envelope.trigger_source = frame.envelope.trigger_source.clone();
        step_frame.envelope.hop = frame.envelope.hop.saturating_add(index as u32 + 1);

        let mut reply_frame = step.site().answer(cx, step_frame)?;
        reply_frame.envelope.role = step.role().cloned().or(reply_frame.envelope.role);
        reply_frame.envelope.hop = frame.envelope.hop.saturating_add(index as u32 + 1);
        let reply = eval_reply_from_frame(cx, &reply_frame)?;
        current_expr = reply.value.object().as_expr(cx)?;
        last_reply = Some(reply);
    }

    let mut final_frame = server_frame_from_reply(
        cx,
        &reply_codec_for_frame(site, &frame),
        last_reply.expect("pipeline steps are non-empty"),
        consistency,
    )?;
    final_frame.msg_id = frame.msg_id;
    final_frame.correlate = frame.correlate;
    final_frame.envelope.reply_codec_hint = frame.envelope.reply_codec_hint;
    final_frame.envelope.role = steps
        .last()
        .and_then(|step| step.role().cloned())
        .or(frame.envelope.role);
    final_frame.envelope.trigger_source = frame.envelope.trigger_source;
    final_frame.envelope.hop = frame.envelope.hop.saturating_add(steps.len() as u32);
    Ok(final_frame)
}

fn answer_pipeline_passthrough(
    cx: &mut Cx,
    steps: &[Connection],
    frame: ServerFrame,
) -> Result<ServerFrame> {
    let mut current = frame;
    for (index, step) in steps.iter().enumerate() {
        current.codec = step.default_codec().clone();
        current.envelope.role = step
            .role()
            .cloned()
            .or_else(|| current.envelope.role.clone());
        current.envelope.hop = current.envelope.hop.saturating_add(1);
        current = step.site().answer(cx, current)?;
        current.envelope.hop = index as u32 + 1;
    }
    Ok(current)
}

fn loop_until_fired(cx: &mut Cx, until: &Value, frame: &ServerFrame) -> Result<bool> {
    let value = match frame.kind {
        FrameKind::Response => eval_reply_from_frame(cx, frame)?.value,
        _ => {
            let expr = frame.decode_expr(cx, ReadPolicy::default())?;
            cx.factory().expr(expr)?
        }
    };
    let fired = cx.call_value(until.clone(), Args::new(vec![value]))?;
    fired.object().truth(cx)
}

fn response_as_request(cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
    let reply = eval_reply_from_frame(cx, &frame)?;
    let request = EvalRequest {
        expr: reply.value.object().as_expr(cx)?,
        result_shape: None,
        required_capabilities: frame.envelope.required_capabilities.clone(),
        deadline: frame.envelope.deadline,
        consistency: frame.envelope.consistency,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: frame.envelope.trace,
    };
    let mut request_frame = server_frame_from_request(cx, &frame.codec, request)?;
    request_frame.msg_id = frame.msg_id;
    request_frame.correlate = frame.correlate;
    request_frame.envelope.reply_codec_hint = frame.envelope.reply_codec_hint;
    request_frame.envelope.role = frame.envelope.role;
    request_frame.envelope.hop = frame.envelope.hop;
    request_frame.envelope.trigger_source = frame.envelope.trigger_source;
    Ok(request_frame)
}

pub(crate) fn eval_request_from_trigger_frame(
    cx: &mut Cx,
    frame: &ServerFrame,
) -> Result<EvalRequest> {
    let expr = frame.decode_expr(cx, ReadPolicy::default())?;
    enforce_trigger_eval_policy(cx, &expr)?;
    Ok(EvalRequest {
        expr,
        result_shape: None,
        required_capabilities: frame.envelope.required_capabilities.clone(),
        deadline: frame.envelope.deadline,
        consistency: frame.envelope.consistency,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: frame.envelope.trace,
    })
}

pub(crate) fn enforce_trigger_eval_policy(cx: &Cx, expr: &Expr) -> Result<()> {
    if is_self_evaluating_trigger_expr(expr) {
        cx.require(&read_eval_capability())?;
    }
    Ok(())
}

fn is_self_evaluating_trigger_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Nil
        | Expr::Bool(_)
        | Expr::Number(_)
        | Expr::Symbol(_)
        | Expr::Local(_)
        | Expr::String(_)
        | Expr::Bytes(_)
        | Expr::List(_)
        | Expr::Vector(_)
        | Expr::Map(_)
        | Expr::Set(_)
        | Expr::Quote { .. } => true,
        Expr::Block(_)
        | Expr::Call { .. }
        | Expr::Infix { .. }
        | Expr::Prefix { .. }
        | Expr::Postfix { .. }
        | Expr::Annotated { .. }
        | Expr::Extension { .. } => false,
    }
}
