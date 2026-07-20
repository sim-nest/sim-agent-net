use super::super::model::AgentComponent;
use super::runner_stream::DiscardModelEventSink;
use super::runner_tools::{run_stream_with_tool_loop, run_with_tool_loop};
use crate::util::{shape_from_expr, shape_protocol, value_from_expr};
use sim_codec_chat::validate_chat_transcript;
use sim_kernel::{Cx, Diagnostic, Error, EvalRequest, Expr, Result, ShapeRef, Symbol};
use sim_lib_agent_runner_core::{
    ModelEventSink, ModelResponse, OUTPUT_GRAMMAR_DIALECT_EXTRA, shape_to_grammar,
};

#[derive(Clone)]
pub(super) struct ShapeContract {
    shape: ShapeRef,
    shape_expr: Expr,
    repair_enabled: bool,
    submit_shape: bool,
}

struct ShapeValidation {
    response: Expr,
    ok: bool,
    diagnostics: Vec<String>,
}

pub(super) fn prepare_shape_contract_request(
    cx: &mut Cx,
    component: &AgentComponent,
    mut request: EvalRequest,
) -> Result<(EvalRequest, Option<ShapeContract>)> {
    let Some((shape, shape_expr)) = shape_contract_shape(cx, component, &request)? else {
        return Ok((request, None));
    };
    if request.result_shape.is_none() {
        request.result_shape = Some(shape.clone());
    }
    let repair_enabled = request_bool_field(&request.expr, "repair").unwrap_or(false);
    let submit_shape = request_bool_field(&request.expr, "submit-shape").unwrap_or(false);
    request.expr = upsert_request_field(request.expr, "output-shape", shape_expr.clone())?;
    if let Ok(grammar) = shape_to_grammar(shape_protocol(&shape)?) {
        request.expr = upsert_request_field(request.expr, "output-grammar", Expr::String(grammar))?;
        request.expr = upsert_request_field(
            request.expr,
            OUTPUT_GRAMMAR_DIALECT_EXTRA,
            Expr::Symbol(Symbol::new("json-schema")),
        )?;
    }
    if submit_shape {
        request.expr = upsert_request_field(request.expr, "submit-shape", Expr::Bool(true))?;
        request.expr = inject_submit_shape_tool(request.expr, &shape_expr)?;
    }
    Ok((
        request,
        Some(ShapeContract {
            shape,
            shape_expr,
            repair_enabled,
            submit_shape,
        }),
    ))
}

pub(super) fn run_with_shape_contract<F>(
    cx: &mut Cx,
    component: &AgentComponent,
    request: EvalRequest,
    mut infer_once: F,
) -> Result<Expr>
where
    F: FnMut(&mut Cx, &EvalRequest) -> Result<Expr>,
{
    let (request, contract) = prepare_shape_contract_request(cx, component, request)?;
    let response = run_with_tool_loop(cx, component, request.clone(), |cx, request| {
        infer_once(cx, request)
    })?;
    finalize_shape_checked_response(cx, component, request, contract, response, infer_once)
}

pub(super) fn run_stream_with_shape_contract<F>(
    cx: &mut Cx,
    component: &AgentComponent,
    request: EvalRequest,
    events: &mut dyn ModelEventSink,
    mut infer_once: F,
) -> Result<ModelResponse>
where
    F: FnMut(&mut Cx, &EvalRequest, &mut dyn ModelEventSink) -> Result<ModelResponse>,
{
    let (request, contract) = prepare_shape_contract_request(cx, component, request)?;
    let response = run_stream_with_tool_loop(
        cx,
        component,
        request.clone(),
        events,
        |cx, request, events| infer_once(cx, request, events),
    )?;
    let expr = finalize_shape_checked_response(
        cx,
        component,
        request,
        contract,
        Expr::from(response),
        |cx, request| {
            let mut events = DiscardModelEventSink;
            infer_once(cx, request, &mut events).map(Expr::from)
        },
    )?;
    ModelResponse::try_from(expr)
}

pub(super) fn finalize_shape_checked_response<F>(
    cx: &mut Cx,
    component: &AgentComponent,
    request: EvalRequest,
    contract: Option<ShapeContract>,
    response: Expr,
    mut infer_once: F,
) -> Result<Expr>
where
    F: FnMut(&mut Cx, &EvalRequest) -> Result<Expr>,
{
    let Some(contract) = contract else {
        return Ok(response);
    };
    let validated = validate_shape_response(cx, &contract, response, false)?;
    if validated.ok {
        return Ok(validated.response);
    }
    if !contract.repair_enabled {
        record_shape_failure(cx, &validated.diagnostics);
        return Ok(validated.response);
    }
    let repair_request = build_repair_request(
        request,
        &contract,
        &validated.response,
        &validated.diagnostics,
    )?;
    let repaired = run_with_tool_loop(cx, component, repair_request, |cx, request| {
        infer_once(cx, request)
    })?;
    let repaired = validate_shape_response(cx, &contract, repaired, true)?;
    if !repaired.ok {
        record_shape_failure(cx, &repaired.diagnostics);
    }
    Ok(repaired.response)
}

fn validate_shape_response(
    cx: &mut Cx,
    contract: &ShapeContract,
    response: Expr,
    repaired: bool,
) -> Result<ShapeValidation> {
    let Some(candidate) = response_candidate_expr(&response) else {
        let diagnostics = vec![if contract.submit_shape {
            "submit_shape_value was required to finish this request".to_owned()
        } else {
            "model response did not carry a shape candidate".to_owned()
        }];
        return Ok(ShapeValidation {
            response: annotate_shape_response(response, false, &diagnostics, repaired)?,
            ok: false,
            diagnostics,
        });
    };
    let candidate_value = value_from_expr(cx, &candidate)?;
    let matched = shape_protocol(&contract.shape)?.check_value(cx, candidate_value)?;
    if matched.accepted {
        return Ok(ShapeValidation {
            response: annotate_shape_response(response, true, &[], repaired)?,
            ok: true,
            diagnostics: Vec::new(),
        });
    }
    let diagnostics = if matched.diagnostics.is_empty() {
        vec![format!("output did not satisfy {:?}", contract.shape_expr)]
    } else {
        matched
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect()
    };
    Ok(ShapeValidation {
        response: annotate_shape_response(response, false, &diagnostics, repaired)?,
        ok: false,
        diagnostics,
    })
}

fn build_repair_request(
    mut request: EvalRequest,
    contract: &ShapeContract,
    response: &Expr,
    diagnostics: &[String],
) -> Result<EvalRequest> {
    let mut messages = request_messages(&request.expr)?;
    messages.push(chat_message(
        "assistant",
        format!("Previous response: {response:?}"),
    ));
    messages.push(chat_message(
        "user",
        format!(
            "The previous response failed output-shape validation for {:?}: {}. Return a value that matches the shape{}.",
            contract.shape_expr,
            diagnostics.join("; "),
            if contract.submit_shape {
                " by calling submit_shape_value exactly once"
            } else {
                ""
            }
        ),
    ));
    request.expr = upsert_request_field(request.expr, "messages", Expr::List(messages))?;
    Ok(request)
}

fn shape_contract_shape(
    cx: &mut Cx,
    component: &AgentComponent,
    request: &EvalRequest,
) -> Result<Option<(ShapeRef, Expr)>> {
    if let Some(shape) = &request.result_shape {
        return Ok(Some((shape.clone(), shape.object().as_expr(cx)?)));
    }
    let Some(expr) = request_field(&request.expr, "output-shape") else {
        return Ok(None);
    };
    Ok(Some((
        shape_from_expr(cx, expr, &component.symbol, "output")?,
        expr.clone(),
    )))
}

fn request_field<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn request_bool_field(expr: &Expr, key: &str) -> Option<bool> {
    match request_field(expr, key) {
        Some(Expr::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn upsert_request_field(mut expr: Expr, key: &str, value: Expr) -> Result<Expr> {
    let Expr::Map(entries) = &mut expr else {
        return Err(Error::Eval("model request must be a map".to_owned()));
    };
    for (field, current) in entries.iter_mut() {
        if matches!(field, Expr::Symbol(symbol) if symbol.name.as_ref() == key) {
            *current = value;
            return Ok(expr);
        }
    }
    entries.push((Expr::Symbol(Symbol::new(key)), value));
    Ok(expr)
}

fn inject_submit_shape_tool(mut request: Expr, shape_expr: &Expr) -> Result<Expr> {
    let Expr::Map(entries) = &mut request else {
        return Err(Error::Eval("model request must be a map".to_owned()));
    };
    let submit_tool = Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("name")),
            Expr::Symbol(Symbol::new("submit_shape_value")),
        ),
        (Expr::Symbol(Symbol::new("args")), shape_expr.clone()),
        (
            Expr::Symbol(Symbol::new("description")),
            Expr::String("finish by submitting one value that matches output-shape".to_owned()),
        ),
    ]);
    for (field, value) in entries.iter_mut() {
        if matches!(field, Expr::Symbol(symbol) if symbol.name.as_ref() == "tools") {
            let Expr::List(items) = value else {
                return Err(Error::Eval(
                    "model request tools field must be a list".to_owned(),
                ));
            };
            let submit_name = Expr::Symbol(Symbol::new("submit_shape_value"));
            if !items.iter().any(|item| {
                matches!(
                    item,
                    Expr::Map(tool_entries)
                        if tool_entries.iter().any(|(key, value)| {
                            *key == Expr::Symbol(Symbol::new("name")) && *value == submit_name
                        })
                )
            }) {
                items.push(submit_tool);
            }
            return Ok(request);
        }
    }
    entries.push((
        Expr::Symbol(Symbol::new("tools")),
        Expr::List(vec![submit_tool]),
    ));
    Ok(request)
}

fn response_candidate_expr(response: &Expr) -> Option<Expr> {
    if let Some(value) = response_field(response, "submitted-value") {
        return Some(value.clone());
    }
    if let Some(Expr::List(content)) = response_field(response, "content")
        && let [Expr::Map(part)] = content.as_slice()
    {
        for (key, value) in part {
            if *key == Expr::Symbol(Symbol::new("text")) && matches!(value, Expr::String(_)) {
                return Some(value.clone());
            }
            if *key == Expr::Symbol(Symbol::new("value")) {
                return Some(value.clone());
            }
        }
    }
    match response_field(response, "text") {
        Some(Expr::String(text)) => Some(Expr::String(text.clone())),
        _ => None,
    }
}

fn response_field<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn annotate_shape_response(
    expr: Expr,
    ok: bool,
    diagnostics: &[String],
    repaired: bool,
) -> Result<Expr> {
    let Expr::Map(mut entries) = expr else {
        return Err(Error::Eval(
            "runner response must be a model-response map".to_owned(),
        ));
    };
    entries.retain(|(key, _)| {
        !matches!(
            key,
            Expr::Symbol(symbol)
                if matches!(
                    symbol.name.as_ref(),
                    "shape-ok" | "shape-diagnostics" | "shape-repaired"
                )
        )
    });
    entries.push((Expr::Symbol(Symbol::new("shape-ok")), Expr::Bool(ok)));
    if repaired {
        entries.push((
            Expr::Symbol(Symbol::new("shape-repaired")),
            Expr::Bool(true),
        ));
    }
    if !diagnostics.is_empty() {
        entries.push((
            Expr::Symbol(Symbol::new("shape-diagnostics")),
            Expr::List(
                diagnostics
                    .iter()
                    .cloned()
                    .map(Expr::String)
                    .collect::<Vec<_>>(),
            ),
        ));
    }
    let out = Expr::Map(entries);
    validate_chat_transcript(&out)?;
    Ok(out)
}

fn record_shape_failure(cx: &mut Cx, diagnostics: &[String]) {
    for diagnostic in diagnostics {
        cx.push_diagnostic(Diagnostic::error(diagnostic.clone()));
    }
}

fn request_messages(expr: &Expr) -> Result<Vec<Expr>> {
    match request_field(expr, "messages") {
        Some(Expr::List(messages)) => Ok(messages.clone()),
        Some(_) => Err(Error::Eval(
            "model request messages field must be a list".to_owned(),
        )),
        None => Ok(Vec::new()),
    }
}

fn chat_message(role: &str, text: String) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("role")),
            Expr::Symbol(Symbol::new(role)),
        ),
        (
            Expr::Symbol(Symbol::new("content")),
            Expr::List(vec![Expr::Map(vec![
                (
                    Expr::Symbol(Symbol::new("type")),
                    Expr::Symbol(Symbol::new("text")),
                ),
                (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
            ])]),
        ),
    ])
}
