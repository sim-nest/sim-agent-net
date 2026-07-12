use super::super::model::{AgentComponent, RunnerBackend};
use super::runner_cache::{CacheEventSink, RunnerCachePlan, normalize_expr, set_cache_hit};
use super::runner_effects::{
    model_infer_replay_key, resolve_model_infer_effect, resolve_model_infer_stream_effect,
};
use super::runner_fake::{fake_response_expr, fake_stream_response};
use super::runner_shape::{run_stream_with_shape_contract, run_with_shape_contract};
use super::runner_stream::{
    FinalEventTracker, ModelEventStreamSink, TeeModelEventSink, model_stream_metadata,
};
use crate::model_privacy::enforce_component_runner_policy;
use crate::util::value_from_expr;
use sim_codec_chat::{model_error_expr, model_response_expr, validate_chat_transcript};
use sim_kernel::{ContentId, Cx, Error, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelRequest, ModelResponse};
use sim_lib_server::{
    FrameKind, ServerFrame, StreamSink, eval_reply_from_frame, eval_request_from_frame,
    server_frame_from_reply,
};
use std::{collections::BTreeMap, sync::Arc};

pub(in crate::components) fn answer_runner(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RunnerBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "{} only answers request frames",
            component.symbol
        )));
    }
    for capability in &component.capabilities {
        cx.require(capability)?;
    }
    let consistency = frame.envelope.consistency;
    let msg_id = frame.msg_id;
    let correlate = frame.correlate;
    let request = eval_request_from_frame(cx, &frame)?;
    if let Some((site, routed_frame)) =
        super::super::placement::routed_model_site_frame(cx, &frame.codec, &request)?
    {
        let routed_reply = site.answer(cx, routed_frame)?;
        let reply = eval_reply_from_frame(cx, &routed_reply)?;
        let mut frame = server_frame_from_reply(cx, &frame.codec, reply, consistency)?;
        frame.msg_id = msg_id;
        frame.correlate = correlate.or(msg_id);
        return Ok(frame);
    }
    enforce_component_runner_policy(component, backend, &request.expr)?;
    let result = run_with_shape_contract(cx, component, request, |cx, request| {
        infer_runner_once(cx, component, backend, request)
    })?;
    let value = value_from_expr(cx, &result)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
}

pub(in crate::components) fn stream_runner(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RunnerBackend,
    frame: ServerFrame,
    sink: &mut dyn StreamSink,
) -> Result<()> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "{} only answers request frames",
            component.symbol
        )));
    }
    for capability in &component.capabilities {
        cx.require(capability)?;
    }
    let request = eval_request_from_frame(cx, &frame)?;
    if let Some((site, routed_frame)) =
        super::super::placement::routed_model_site_frame(cx, &frame.codec, &request)?
    {
        return site.stream(cx, routed_frame, sink);
    }
    enforce_component_runner_policy(component, backend, &request.expr)?;
    let codec = frame.codec.clone();
    let envelope = frame.envelope.clone();
    {
        let mut stream_events = ModelEventStreamSink::new(cx, codec, envelope, sink);
        stream_events.emit_start(model_stream_metadata(
            component.symbol.clone(),
            runner_model_id(backend),
        ))?;
        let mut final_tracker = FinalEventTracker::default();
        let response = {
            let mut events = TeeModelEventSink::new(&mut stream_events, &mut final_tracker);
            run_stream_with_shape_contract(
                cx,
                component,
                request,
                &mut events,
                |cx, request, events| {
                    infer_runner_once_stream(cx, component, backend, request, events)
                },
            )?
        };
        if !final_tracker.seen_final() {
            let mut events = TeeModelEventSink::new(&mut stream_events, &mut final_tracker);
            events.emit(ModelEvent::final_of(&response))?;
        }
        stream_events.emit_end()?;
    }
    sink.end(cx)
}

fn infer_runner_once(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RunnerBackend,
    request: &EvalRequest,
) -> Result<Expr> {
    let model_id = runner_model_id(backend);
    let cache = RunnerCachePlan::prepare(cx, request, &component.symbol, &model_id)?;
    if let Some(hit) = cache.hit() {
        return Ok(hit);
    }
    cache.require_write_capability(cx)?;
    let response = resolve_model_infer_effect(cx, request, |cx| {
        infer_runner_once_uncached(cx, component, backend, request)
    })?;
    cache.finish(response)
}

fn infer_runner_once_uncached(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RunnerBackend,
    request: &EvalRequest,
) -> Result<Expr> {
    match backend {
        RunnerBackend::Echo { model } => {
            text_response_expr(component, model, request_task_text(&request.expr))
        }
        RunnerBackend::Cassette {
            model,
            strict,
            entries,
        } => cassette_response_expr(cx, component, model, *strict, entries, request),
        RunnerBackend::Fake {
            model,
            script,
            delay,
        } => fake_response_expr(component, model, script, *delay),
        RunnerBackend::External { runner } => {
            let model_request = ModelRequest::try_from(request.expr.clone())?;
            let response = runner.infer(cx, model_request)?;
            normalize_external_response(response)
        }
    }
}

fn infer_runner_once_stream(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RunnerBackend,
    request: &EvalRequest,
    events: &mut dyn ModelEventSink,
) -> Result<ModelResponse> {
    let model_id = runner_model_id(backend);
    let cache = RunnerCachePlan::prepare(cx, request, &component.symbol, &model_id)?;
    if let Some(hit) = cache.hit() {
        let response = ModelResponse::try_from(hit)?;
        events.emit(ModelEvent::final_of(&response))?;
        return Ok(response);
    }
    cache.require_write_capability(cx)?;
    if cache.is_active() {
        let mut cache_events = CacheEventSink::new(events);
        let response =
            resolve_model_infer_stream_effect(cx, request, &mut cache_events, |cx, events| {
                infer_runner_once_stream_uncached(cx, component, backend, request, events)
            })?;
        let response = ModelResponse::try_from(cache.finish(response.into())?)?;
        events.emit(ModelEvent::final_of(&response))?;
        return Ok(response);
    }
    resolve_model_infer_stream_effect(cx, request, events, |cx, events| {
        infer_runner_once_stream_uncached(cx, component, backend, request, events)
    })
}

fn infer_runner_once_stream_uncached(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RunnerBackend,
    request: &EvalRequest,
    events: &mut dyn ModelEventSink,
) -> Result<ModelResponse> {
    match backend {
        RunnerBackend::Echo { model } => {
            let expr = text_response_expr(component, model, request_task_text(&request.expr))?;
            let response = ModelResponse::try_from(expr)?;
            events.emit(ModelEvent::final_of(&response))?;
            Ok(response)
        }
        RunnerBackend::Cassette {
            model,
            strict,
            entries,
        } => {
            let expr = cassette_response_expr(cx, component, model, *strict, entries, request)?;
            let response = ModelResponse::try_from(expr)?;
            events.emit(ModelEvent::final_of(&response))?;
            Ok(response)
        }
        RunnerBackend::Fake {
            model,
            script,
            delay,
        } => fake_stream_response(component, model, script, *delay, events),
        RunnerBackend::External { runner } => {
            let model_request = ModelRequest::try_from(request.expr.clone())?;
            runner.infer_stream(cx, model_request, events)
        }
    }
}

fn runner_model_id(backend: &RunnerBackend) -> String {
    match backend {
        RunnerBackend::Echo { model }
        | RunnerBackend::Cassette { model, .. }
        | RunnerBackend::Fake { model, .. } => model.clone(),
        RunnerBackend::External { runner } => runner.card().model,
    }
}

fn text_response_expr(component: &AgentComponent, model: &str, text: String) -> Result<Expr> {
    let part = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("type")),
            Expr::Symbol(Symbol::new("text")),
        ),
        (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
    ]);
    Ok(model_response_expr(
        component.symbol.clone(),
        model,
        vec![part],
        Symbol::new("stop"),
    ))
}

fn cassette_response_expr(
    cx: &mut Cx,
    component: &AgentComponent,
    model: &str,
    strict: bool,
    entries: &Arc<Vec<Expr>>,
    request: &EvalRequest,
) -> Result<Expr> {
    if let Some(response) = cassette_lookup(cx, entries, request)? {
        return set_cache_hit(response, true);
    }
    if strict {
        return set_cache_hit(
            model_error_expr(
                component.symbol.clone(),
                model,
                format!("cassette miss for {}", request_task_text(&request.expr)),
            ),
            false,
        );
    }
    set_cache_hit(
        text_response_expr(component, model, request_task_text(&request.expr))?,
        false,
    )
}

fn normalize_external_response(response: ModelResponse) -> Result<Expr> {
    let expr: Expr = response.into();
    validate_chat_transcript(&expr)?;
    Ok(expr)
}

fn cassette_lookup(cx: &mut Cx, entries: &[Expr], request: &EvalRequest) -> Result<Option<Expr>> {
    let wanted = live_cassette_key(cx, request)?;
    let mut pending = BTreeMap::<String, ContentId>::new();
    for entry in entries {
        if let Some((task_id, key)) = recorded_request(cx, entry)? {
            pending.insert(task_id, key);
            continue;
        }
        if let Some((task_id, response)) = recorded_response(entry)?
            && pending
                .get(&task_id)
                .is_some_and(|recorded_key| recorded_key == &wanted)
        {
            return Ok(Some(response));
        }
    }
    Ok(None)
}

fn recorded_request(cx: &mut Cx, entry: &Expr) -> Result<Option<(String, ContentId)>> {
    if !trace_kind_matches(entry, "request") {
        return Ok(None);
    }
    let Some(task_id) = trace_task_id(entry) else {
        return Ok(None);
    };
    let Some(payload) = trace_field(entry, "payload") else {
        return Ok(None);
    };
    if let Some(request_expr) = eval_request_expr(payload) {
        return Ok(Some((
            task_id,
            recorded_cassette_key(cx, request_expr, eval_request_shape(payload))?,
        )));
    }
    if ModelRequest::try_from(payload.clone()).is_ok() {
        return Ok(Some((task_id, recorded_cassette_key(cx, payload, None)?)));
    }
    Ok(None)
}

fn recorded_response(entry: &Expr) -> Result<Option<(String, Expr)>> {
    if !trace_kind_matches(entry, "response") {
        return Ok(None);
    }
    let Some(task_id) = trace_task_id(entry) else {
        return Ok(None);
    };
    let Some(payload) = trace_field(entry, "payload") else {
        return Ok(None);
    };
    if let Some(value) = eval_reply_value(payload)
        && validate_chat_transcript(value).is_ok()
    {
        return Ok(Some((task_id, value.clone())));
    }
    if validate_chat_transcript(payload).is_ok() {
        return Ok(Some((task_id, payload.clone())));
    }
    Ok(None)
}

fn live_cassette_key(cx: &mut Cx, request: &EvalRequest) -> Result<ContentId> {
    let result_shape = request
        .result_shape
        .as_ref()
        .map(|shape| shape.object().as_expr(cx))
        .transpose()?;
    recorded_cassette_key(cx, &request.expr, result_shape.as_ref())
}

fn recorded_cassette_key(
    cx: &mut Cx,
    expr: &Expr,
    result_shape: Option<&Expr>,
) -> Result<ContentId> {
    let normalized_expr = normalize_expr(expr);
    let normalized_shape = result_shape.map(normalize_expr);
    model_infer_replay_key(cx, &normalized_expr, normalized_shape.as_ref())
}

fn eval_request_expr(payload: &Expr) -> Option<&Expr> {
    payload_field(payload, "expr")
}

fn eval_request_shape(payload: &Expr) -> Option<&Expr> {
    payload_field(payload, "result-shape").filter(|expr| !matches!(expr, Expr::Nil))
}

fn eval_reply_value(payload: &Expr) -> Option<&Expr> {
    payload_field(payload, "value")
}

fn payload_field<'a>(payload: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = payload else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn trace_field<'a>(entry: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = entry else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn trace_kind_matches(entry: &Expr, kind: &str) -> bool {
    matches!(
        trace_field(entry, "kind"),
        Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == kind
    )
}

fn trace_task_id(entry: &Expr) -> Option<String> {
    match trace_field(entry, "task-id") {
        Some(Expr::String(task_id)) => Some(task_id.clone()),
        _ => None,
    }
}

fn request_task_text(expr: &Expr) -> String {
    if let Expr::Map(entries) = expr {
        for (key, value) in entries {
            if *key == Expr::Symbol(Symbol::new("task")) {
                return expr_text(value);
            }
        }
    }
    expr_text(expr)
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(value) => value.to_string(),
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        _ => format!("{expr:?}"),
    }
}
