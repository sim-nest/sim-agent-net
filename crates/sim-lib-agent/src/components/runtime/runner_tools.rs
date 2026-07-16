use super::super::model::AgentComponent;
use super::runner_tool_schema::{
    ToolCall, append_tool_messages, bool_field, declared_tools, key_expr, max_tool_rounds,
    response_model, response_runner, tool_call_fingerprint, tool_calls_from_response,
    tool_result_text,
};
use crate::{
    model_privacy::PrivacyPolicy,
    tools::resolve_tool_by_symbol,
    util::{shape_protocol, value_from_expr},
};
use sim_codec_chat::model_error_expr;
use sim_kernel::{
    Cx, Datum, DatumStore, Diagnostic, Effect, Error, EvalRequest, Expr, Ref, RefResolver, Result,
    ShapeRef, Symbol, TemporaryRefResolver, core_any_ref, effect, value_from_ref,
};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelResponse};
use std::collections::BTreeSet;

fn tool_call_effect_kind() -> Symbol {
    Symbol::qualified("effect", "tool-call")
}

pub(super) fn run_with_tool_loop<F>(
    cx: &mut Cx,
    component: &AgentComponent,
    request: EvalRequest,
    mut infer_once: F,
) -> Result<Expr>
where
    F: FnMut(&mut Cx, &EvalRequest) -> Result<Expr>,
{
    let mut state = ToolLoopState::new(&request)?;
    let mut request = request;
    loop {
        let response = infer_once(cx, &request)?;
        let tool_calls = match tool_calls_from_response(&response) {
            Ok(tool_calls) => tool_calls,
            Err(err) => {
                return Ok(model_error_for_response(
                    component,
                    &response,
                    err.to_string(),
                ));
            }
        };
        if tool_calls.is_empty() {
            return Ok(response);
        }
        if state.rounds_used >= state.max_rounds {
            return Ok(model_error_for_response(
                component,
                &response,
                format!(
                    "tool turn budget exhausted after {} rounds",
                    state.max_rounds
                ),
            ));
        }
        match run_tool_round(cx, component, &response, tool_calls, &mut state, None)? {
            ToolRound::Continue(messages) => {
                request.expr = append_tool_messages(request.expr, messages)?;
                state.rounds_used += 1;
            }
            ToolRound::Submitted(response) => return Ok(response),
            ToolRound::Fatal(message) => {
                return Ok(model_error_for_response(component, &response, message));
            }
        }
    }
}

pub(super) fn run_stream_with_tool_loop<F>(
    cx: &mut Cx,
    component: &AgentComponent,
    request: EvalRequest,
    events: &mut dyn ModelEventSink,
    mut infer_once: F,
) -> Result<ModelResponse>
where
    F: FnMut(&mut Cx, &EvalRequest, &mut dyn ModelEventSink) -> Result<ModelResponse>,
{
    let mut state = ToolLoopState::new(&request)?;
    let mut request = request;
    loop {
        let response = infer_once(cx, &request, events)?;
        let response_expr = Expr::from(response.clone());
        let tool_calls = match tool_calls_from_response(&response_expr) {
            Ok(tool_calls) => tool_calls,
            Err(err) => {
                return emit_stream_error(component, &response_expr, err.to_string(), events);
            }
        };
        if tool_calls.is_empty() {
            return Ok(response);
        }
        if state.rounds_used >= state.max_rounds {
            return emit_stream_error(
                component,
                &response_expr,
                format!(
                    "tool turn budget exhausted after {} rounds",
                    state.max_rounds
                ),
                events,
            );
        }
        match run_tool_round(
            cx,
            component,
            &response_expr,
            tool_calls,
            &mut state,
            Some(events),
        )? {
            ToolRound::Continue(messages) => {
                request.expr = append_tool_messages(request.expr, messages)?;
                state.rounds_used += 1;
            }
            ToolRound::Submitted(response) => return ModelResponse::try_from(response),
            ToolRound::Fatal(message) => {
                return emit_stream_error(component, &response_expr, message, events);
            }
        }
    }
}

#[derive(Clone)]
struct ToolLoopState {
    declared_tools: Option<BTreeSet<Symbol>>,
    seen_calls: BTreeSet<String>,
    privacy_policy: PrivacyPolicy,
    submit_shape: Option<ShapeRef>,
    allow_repeated: bool,
    max_rounds: u32,
    rounds_used: u32,
}

impl ToolLoopState {
    fn new(request: &EvalRequest) -> Result<Self> {
        let mut declared = declared_tools(&request.expr)?;
        let submit_shape = bool_field(&request.expr, "submit-shape")
            .unwrap_or(false)
            .then(|| request.result_shape.clone())
            .flatten();
        if submit_shape.is_some() {
            declared
                .get_or_insert_with(BTreeSet::new)
                .insert(submit_shape_tool());
        }
        Ok(Self {
            declared_tools: declared,
            seen_calls: BTreeSet::new(),
            privacy_policy: PrivacyPolicy::from_request_expr(&request.expr)?,
            submit_shape,
            allow_repeated: bool_field(&request.expr, "allow-repeated-tool-calls").unwrap_or(false),
            max_rounds: max_tool_rounds(&request.expr)?,
            rounds_used: 0,
        })
    }
}

enum ToolRound {
    Continue(Vec<Expr>),
    Submitted(Expr),
    Fatal(String),
}

fn run_tool_round(
    cx: &mut Cx,
    component: &AgentComponent,
    response: &Expr,
    tool_calls: Vec<ToolCall>,
    state: &mut ToolLoopState,
    mut events: Option<&mut dyn ModelEventSink>,
) -> Result<ToolRound> {
    let mut messages = Vec::new();
    let model = response_model(response).unwrap_or_else(|| component.symbol.to_string());
    for call in tool_calls {
        if call.name == submit_shape_tool() {
            let Some(shape) = state.submit_shape.as_ref() else {
                return Ok(ToolRound::Fatal(
                    "submit_shape_value was not declared on the model request".to_owned(),
                ));
            };
            let outcome = submit_shape_outcome(cx, shape, &call);
            if let Some(sink) = events.as_deref_mut() {
                sink.emit(tool_result_event(component, &model, &call, &outcome))?;
            }
            if outcome.error {
                return Ok(ToolRound::Fatal(outcome.text));
            }
            let submitted =
                call.args.first().cloned().ok_or_else(|| {
                    Error::Eval("submit_shape_value expects one argument".to_owned())
                })?;
            return Ok(ToolRound::Submitted(submit_shape_response(
                component, response, submitted,
            )));
        }
        if !state
            .declared_tools
            .as_ref()
            .is_some_and(|tools| tools.contains(&call.name))
        {
            return Ok(ToolRound::Fatal(format!(
                "tool {} was not declared on the model request",
                call.name
            )));
        }
        if !state.privacy_policy.allows_tool_content(&call.name) {
            return Ok(ToolRound::Fatal(format!(
                "privacy policy denied tool {}",
                call.name
            )));
        }
        let fingerprint = tool_call_fingerprint(&call);
        if !state.allow_repeated && !state.seen_calls.insert(fingerprint) {
            return Ok(ToolRound::Fatal(format!(
                "repeated tool call {} rejected",
                call.name
            )));
        }
        if let Some(sink) = events.as_deref_mut() {
            sink.emit(ModelEvent::tool_call(
                component.symbol.clone(),
                model.clone(),
                call.id.clone(),
                call.raw.clone(),
            ))?;
        }
        let tool = match resolve_tool_by_symbol(cx, &call.name) {
            Ok(tool) => tool,
            Err(err) => {
                return Ok(ToolRound::Fatal(format!(
                    "unknown tool {}: {err}",
                    call.name
                )));
            }
        };
        let outcome = call_tool(cx, &tool, &call);
        if let Some(sink) = events.as_deref_mut() {
            sink.emit(tool_result_event(component, &model, &call, &outcome))?;
        }
        messages.push(tool_result_message(&call, &outcome));
    }
    Ok(ToolRound::Continue(messages))
}

fn call_tool(cx: &mut Cx, tool: &crate::Tool, call: &ToolCall) -> ToolOutcome {
    let args = match call
        .args
        .iter()
        .map(|arg| value_from_expr(cx, arg))
        .collect::<Result<Vec<_>>>()
    {
        Ok(args) => args,
        Err(err) => return ToolOutcome::error(cx, call, err),
    };
    let effect = match tool_call_effect(cx, tool, call) {
        Ok(effect) => effect,
        Err(err) => return ToolOutcome::error(cx, call, err),
    };
    let result = effect::resolve_effect(cx, effect, |cx, _effect| {
        let value = tool.call_values(cx, args)?;
        TemporaryRefResolver::new().ref_for_value(cx, &value)
    })
    .and_then(|reference| value_from_ref(cx, &reference));
    match result {
        Ok(value) => match value.object().as_expr(cx) {
            Ok(expr) => ToolOutcome::success(expr),
            Err(err) => ToolOutcome::error(cx, call, err),
        },
        Err(err) => ToolOutcome::error(cx, call, err),
    }
}

fn tool_call_effect(cx: &mut Cx, tool: &crate::Tool, call: &ToolCall) -> Result<Effect> {
    let input = tool_call_input_datum(tool, call)?;
    let input_ref = Ref::Content(cx.datum_store_mut().intern(input)?);
    let implementation = Ref::Symbol(Symbol::qualified("agent", "tool-call-v1"));
    let result_shape = match &tool.result_shape {
        Some(shape) => TemporaryRefResolver::new().ref_for_value(cx, shape)?,
        None => core_any_ref(),
    };
    Effect::new(
        tool_call_effect_kind(),
        Ref::Symbol(tool.symbol.clone()),
        input_ref,
        result_shape,
        effect::effect_resume_op_key(),
        effect::effect_abort_op_key(),
    )
    .with_requirements(tool.capabilities.clone())
    .with_replay_key(Some(implementation))
}

fn tool_call_input_datum(tool: &crate::Tool, call: &ToolCall) -> Result<Datum> {
    let mut capabilities = tool.capabilities.clone();
    capabilities.sort();
    capabilities.dedup();
    let args = call
        .args
        .iter()
        .cloned()
        .map(Datum::try_from)
        .collect::<Result<Vec<_>>>()?;
    Ok(Datum::Node {
        tag: Symbol::qualified("agent", "ToolCallInput"),
        fields: vec![
            (Symbol::new("tool"), Datum::Symbol(call.name.clone())),
            (Symbol::new("arguments"), Datum::List(args)),
            (
                Symbol::new("implementation"),
                Datum::String("sim-lib-agent-tool-call-v1".to_owned()),
            ),
            (
                Symbol::new("capabilities"),
                Datum::List(
                    capabilities
                        .into_iter()
                        .map(|capability| Datum::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            ),
        ],
    })
}

fn submit_shape_outcome(cx: &mut Cx, shape: &ShapeRef, call: &ToolCall) -> ToolOutcome {
    let [submitted] = call.args.as_slice() else {
        return ToolOutcome::error_text("submit_shape_value expects exactly one argument");
    };
    let value = match value_from_expr(cx, submitted) {
        Ok(value) => value,
        Err(err) => return ToolOutcome::error_text(format!("submit_shape_value failed: {err}")),
    };
    let matched = match shape_protocol(shape).and_then(|shape| shape.check_value(cx, value)) {
        Ok(matched) => matched,
        Err(err) => return ToolOutcome::error_text(format!("submit_shape_value failed: {err}")),
    };
    if matched.accepted {
        ToolOutcome::success(submitted.clone())
    } else {
        ToolOutcome::error_text(join_shape_diagnostics(&matched.diagnostics))
    }
}

struct ToolOutcome {
    text: String,
    expr: Option<Expr>,
    error: bool,
}

impl ToolOutcome {
    fn success(expr: Expr) -> Self {
        Self {
            text: tool_result_text(&expr),
            expr: Some(expr),
            error: false,
        }
    }

    fn error(cx: &mut Cx, call: &ToolCall, err: Error) -> Self {
        let text = format!("tool {} failed: {err}", call.name);
        cx.push_diagnostic(Diagnostic::error(text.clone()));
        Self {
            text,
            expr: None,
            error: true,
        }
    }

    fn error_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            expr: None,
            error: true,
        }
    }
}

fn tool_result_message(call: &ToolCall, outcome: &ToolOutcome) -> Expr {
    let mut message = vec![
        key_expr("role", Expr::Symbol(Symbol::new("tool"))),
        key_expr("name", Expr::Symbol(call.name.clone())),
        key_expr("tool-call-id", call.id.clone()),
        key_expr(
            "content",
            Expr::List(vec![tool_result_content_part(outcome)]),
        ),
    ];
    if outcome.error {
        message.push(key_expr("error", Expr::Bool(true)));
    }
    Expr::Map(message)
}

fn tool_result_content_part(outcome: &ToolOutcome) -> Expr {
    let mut entries = vec![
        key_expr("type", Expr::Symbol(Symbol::new("text"))),
        key_expr("text", Expr::String(outcome.text.clone())),
    ];
    if let Some(expr) = &outcome.expr {
        entries.push(key_expr("expr", expr.clone()));
    }
    Expr::Map(entries)
}

fn tool_result_event(
    component: &AgentComponent,
    model: &str,
    call: &ToolCall,
    outcome: &ToolOutcome,
) -> ModelEvent {
    let mut event = ModelEvent::new(
        Symbol::new("tool-result"),
        component.symbol.clone(),
        model.to_owned(),
        call.id.clone(),
    )
    .with_field("tool", Expr::Symbol(call.name.clone()))
    .with_field("tool-call-id", call.id.clone())
    .with_field("text", Expr::String(outcome.text.clone()));
    if let Some(expr) = &outcome.expr {
        event = event.with_field("result", expr.clone());
    }
    if outcome.error {
        event = event.with_field("error", Expr::Bool(true));
    }
    event
}

fn emit_stream_error(
    component: &AgentComponent,
    response: &Expr,
    message: String,
    events: &mut dyn ModelEventSink,
) -> Result<ModelResponse> {
    let expr = model_error_for_response(component, response, message.clone());
    let model_response = ModelResponse::try_from(expr)?;
    events.emit(ModelEvent::error_text(
        model_response.runner.clone(),
        model_response.model.clone(),
        Expr::Symbol(Symbol::new("tool-loop")),
        message,
    ))?;
    events.emit(ModelEvent::final_of(&model_response))?;
    Ok(model_response)
}

fn model_error_for_response(
    component: &AgentComponent,
    response: &Expr,
    message: impl Into<String>,
) -> Expr {
    model_error_expr(
        response_runner(response).unwrap_or_else(|| component.symbol.clone()),
        response_model(response).unwrap_or_else(|| component.symbol.to_string()),
        message,
    )
}

fn submit_shape_response(component: &AgentComponent, response: &Expr, submitted: Expr) -> Expr {
    Expr::Map(vec![
        key_expr("model-response", Expr::Bool(true)),
        key_expr(
            "runner",
            Expr::Symbol(response_runner(response).unwrap_or_else(|| component.symbol.clone())),
        ),
        key_expr(
            "model",
            Expr::String(response_model(response).unwrap_or_else(|| component.symbol.to_string())),
        ),
        key_expr(
            "content",
            Expr::List(vec![Expr::Map(vec![
                key_expr("type", Expr::Symbol(Symbol::new("expr"))),
                key_expr("value", submitted.clone()),
            ])]),
        ),
        key_expr("stop-reason", Expr::Symbol(Symbol::new("submit-shape"))),
        key_expr("submitted-value", submitted),
    ])
}

fn submit_shape_tool() -> Symbol {
    Symbol::new("submit_shape_value")
}

fn join_shape_diagnostics(diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        "submit_shape_value rejected the value".to_owned()
    } else {
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
            .join("; ")
    }
}
