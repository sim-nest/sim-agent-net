use std::collections::BTreeSet;

use serde_json::{Map, Value};
use sim_kernel::{Args, Cx, Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelResponse, fenced_data_text};

use crate::{
    capabilities::openai_gateway_tools_capability,
    plan::{PlanEvalEvent, PlanEvalReport, eval_plan_report_with_cache},
    runtime::OpenAiPlanCache,
    translate::tools::{OpenAiToolCall, OpenAiToolRegistry, OpenAiToolResult},
};

/// Bounds on the multi-round tool-call loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolLoopConfig {
    /// Maximum number of tool rounds before the loop fails with an error.
    pub max_tool_rounds: usize,
    /// Maximum number of tool calls accepted within a single round.
    pub max_tool_calls_per_round: usize,
}

impl Default for ToolLoopConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: 4,
            max_tool_calls_per_round: 8,
        }
    }
}

/// Runs the tool loop using the tool registry parsed from `request_object`.
///
/// Returns `initial` unchanged when the request declares no tools; otherwise
/// drives [`run_tool_loop_with_registry`] with the default [`ToolLoopConfig`].
pub fn run_tool_loop_with_cache(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    request_object: &Map<String, Value>,
    cache: &mut OpenAiPlanCache,
    initial: PlanEvalReport,
) -> Result<PlanEvalReport> {
    let registry = OpenAiToolRegistry::from_request(cx, request_object)?;
    if registry.is_empty() {
        return Ok(initial);
    }
    run_tool_loop_with_registry(
        cx,
        plan,
        request,
        cache,
        &registry,
        initial,
        ToolLoopConfig::default(),
    )
}

/// Drives the model/tool loop until the model stops requesting tool calls.
///
/// Each round executes the model's requested tool calls against `registry`,
/// appends their results to the request, and re-evaluates `plan`. The loop
/// rejects duplicate identical calls, enforces the per-round call limit, and
/// fails once `config.max_tool_rounds` is exceeded.
pub fn run_tool_loop_with_registry(
    cx: &mut Cx,
    plan: &Expr,
    request: &Expr,
    cache: &mut OpenAiPlanCache,
    registry: &OpenAiToolRegistry,
    initial: PlanEvalReport,
    config: ToolLoopConfig,
) -> Result<PlanEvalReport> {
    let mut events = initial.events;
    let mut current_request = request.clone();
    let mut response = ModelResponse::try_from(initial.response)?;
    let mut seen_calls = BTreeSet::new();

    for _round in 0..config.max_tool_rounds {
        let calls = tool_calls(&response)?;
        if calls.is_empty() {
            return Ok(PlanEvalReport {
                response: Expr::from(response),
                events,
            });
        }
        if calls.len() > config.max_tool_calls_per_round {
            return Err(Error::Eval(format!(
                "tool round requested {} calls, maximum is {}",
                calls.len(),
                config.max_tool_calls_per_round
            )));
        }

        for call in calls {
            let fingerprint = call.fingerprint();
            if !seen_calls.insert(fingerprint) {
                return Err(Error::Eval(format!(
                    "repeated identical tool call rejected: {}",
                    call.name
                )));
            }
            events.push(PlanEvalEvent {
                kind: Symbol::new("tool-call"),
                payload: call.to_expr(),
            });
            let result = execute_tool_call(cx, registry, &call)?;
            events.push(PlanEvalEvent {
                kind: Symbol::new("tool-result"),
                payload: result.to_expr(),
            });
            append_tool_result(&mut current_request, &result)?;
        }

        let next = eval_plan_report_with_cache(cx, plan, &current_request, cache)?;
        events.extend(next.events);
        response = ModelResponse::try_from(next.response)?;
    }

    Err(Error::Eval(format!(
        "tool loop exceeded maximum rounds {}",
        config.max_tool_rounds
    )))
}

fn execute_tool_call(
    cx: &mut Cx,
    registry: &OpenAiToolRegistry,
    call: &OpenAiToolCall,
) -> Result<OpenAiToolResult> {
    let Some(tool) = registry.get(&call.name) else {
        return Ok(OpenAiToolResult::unknown_tool(call));
    };
    if let Err(err) = cx.require(&openai_gateway_tools_capability()) {
        return Ok(OpenAiToolResult::capability_denied(call, err.to_string()));
    }
    if let Err(err) = cx.require_all(tool.capabilities()) {
        return Ok(OpenAiToolResult::capability_denied(call, err.to_string()));
    }
    let args = match tool.argument_values(cx, &call.arguments) {
        Ok(values) => values,
        Err(message) => return Ok(OpenAiToolResult::invalid_arguments(call, message)),
    };
    if let Some(message) = validate_callable_args_shape(cx, tool.symbol(), &args)? {
        return Ok(OpenAiToolResult::invalid_arguments(call, message));
    }
    match cx.call_function(tool.symbol(), Args::new(args)) {
        Ok(value) => value
            .object()
            .as_expr(cx)
            .map(|expr| OpenAiToolResult::success(call, expr)),
        Err(Error::CapabilityDenied { capability }) => Ok(OpenAiToolResult::capability_denied(
            call,
            format!("capability denied: {capability}"),
        )),
        Err(err) => Err(err),
    }
}

fn validate_callable_args_shape(
    cx: &mut Cx,
    symbol: &Symbol,
    args: &[sim_kernel::Value],
) -> Result<Option<String>> {
    let function = cx.resolve_function(symbol)?;
    let Some(callable) = function.object().as_callable() else {
        return Err(Error::TypeMismatch {
            expected: "callable",
            found: "non-callable",
        });
    };
    let Some(shape) = callable.browse_args_shape(cx)? else {
        return Ok(None);
    };
    let Some(shape_impl) = shape.object().as_shape() else {
        return Err(Error::TypeMismatch {
            expected: "shape",
            found: "non-shape",
        });
    };
    let arg_list = cx.factory().list(args.to_vec())?;
    let matched = shape_impl.check_value(cx, arg_list)?;
    Ok((!matched.accepted).then(|| shape_error_message(&matched.diagnostics)))
}

fn shape_error_message(diagnostics: &[sim_kernel::Diagnostic]) -> String {
    diagnostics
        .first()
        .map(|diagnostic| diagnostic.message.clone())
        .unwrap_or_else(|| "callable args shape rejected tool arguments".to_owned())
}

fn tool_calls(response: &ModelResponse) -> Result<Vec<OpenAiToolCall>> {
    response
        .content
        .iter()
        .filter_map(|part| OpenAiToolCall::from_content_part(part).transpose())
        .collect()
}

fn append_tool_result(request: &mut Expr, result: &OpenAiToolResult) -> Result<()> {
    let Expr::Map(entries) = request else {
        return Ok(());
    };
    let message = tool_result_message(result)?;
    if let Some((_, Expr::List(messages))) = entries.iter_mut().find(|(key, _)| {
        matches!(
            key,
            Expr::Symbol(symbol)
                if symbol.namespace.is_none() && symbol.name.as_ref() == "messages"
        )
    }) {
        messages.push(message);
    } else {
        entries.push((
            Expr::Symbol(Symbol::new("messages")),
            Expr::List(vec![message]),
        ));
    }
    Ok(())
}

fn tool_result_message(result: &OpenAiToolResult) -> Result<Expr> {
    let result_expr = result.to_expr();
    let text = fenced_data_text("openai-tool-result", &result.message_text(), &result_expr)?;
    Ok(Expr::Map(vec![
        field("role", Expr::Symbol(Symbol::new("tool"))),
        field("tool-call-id", Expr::String(result.call_id.clone())),
        field(
            "content",
            Expr::List(vec![Expr::Map(vec![
                field("type", Expr::Symbol(Symbol::new("text"))),
                field("text", Expr::String(text)),
            ])]),
        ),
    ]))
}

use sim_value::build::entry as field;
