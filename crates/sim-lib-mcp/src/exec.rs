use std::time::Duration;

#[cfg(feature = "stream")]
use sim_codec_mcp::McpEnvelope;
use sim_kernel::{
    Args, CapabilityName, Consistency, Cx, Error, EvalMode, EvalRequest, Expr, Result, ShapeId,
    ShapeRef, Symbol, Value,
};
use sim_shape::check_value_report;

use crate::content::{McpCallParams, McpToolResult, arguments_to_values};
use crate::{McpSession, McpSurfaceCard, McpSurfaceRole, project_mcp_surface};

/// Returns the capability gating `tools/call` execution.
pub fn mcp_tools_call_capability() -> CapabilityName {
    CapabilityName::new("mcp.tools.call")
}

/// Returns the capability gating `resources/read` execution.
pub fn mcp_resources_read_capability() -> CapabilityName {
    CapabilityName::new("mcp.resources.read")
}

/// Returns the capability gating `prompts/get` execution.
pub fn mcp_prompts_get_capability() -> CapabilityName {
    CapabilityName::new("mcp.prompts.get")
}

pub(crate) fn execute_tool_call(
    cx: &mut Cx,
    session: &McpSession,
    params: McpCallParams,
) -> Result<McpToolResult> {
    let row = resolve_tool_row(cx, session, &params.name)?;
    execute_surface_call(
        cx,
        session,
        &row,
        mcp_tools_call_capability(),
        params.arguments,
        "MCP tool",
    )
}

#[cfg(feature = "stream")]
pub(crate) fn execute_tool_call_with_stream(
    cx: &mut Cx,
    session: &mut McpSession,
    params: McpCallParams,
    progress_token: Option<&Expr>,
) -> Result<(McpToolResult, Vec<McpEnvelope>)> {
    let row = resolve_tool_row(cx, session, &params.name)?;
    execute_surface_call_with_stream(
        cx,
        session,
        &row,
        mcp_tools_call_capability(),
        params.arguments,
        "MCP tool",
        progress_token,
    )
}

pub(crate) fn execute_surface_call(
    cx: &mut Cx,
    session: &McpSession,
    row: &McpSurfaceCard,
    gate_capability: CapabilityName,
    arguments: Vec<Expr>,
    label: &'static str,
) -> Result<McpToolResult> {
    let request = eval_request_for_row(
        row,
        gate_capability,
        arguments.clone(),
        session.deadline_ms,
        label,
    )?;
    require_session_capabilities(session, &request.required_capabilities)?;
    let values = arguments_to_values(cx, &arguments)?;
    validate_input(cx, row.input_shape.as_ref(), &values)?;
    match call_local(cx, row, &request, values) {
        Ok(value) => McpToolResult::success(cx, value),
        Err(error) => Ok(McpToolResult::error(error.to_string())),
    }
}

#[cfg(feature = "stream")]
pub(crate) fn execute_surface_call_with_stream(
    cx: &mut Cx,
    session: &mut McpSession,
    row: &McpSurfaceCard,
    gate_capability: CapabilityName,
    arguments: Vec<Expr>,
    label: &'static str,
    progress_token: Option<&Expr>,
) -> Result<(McpToolResult, Vec<McpEnvelope>)> {
    let request = eval_request_for_row(
        row,
        gate_capability,
        arguments.clone(),
        session.deadline_ms,
        label,
    )?;
    require_session_capabilities(session, &request.required_capabilities)?;
    let values = arguments_to_values(cx, &arguments)?;
    validate_input(cx, row.input_shape.as_ref(), &values)?;
    match call_local(cx, row, &request, values) {
        Ok(value) => {
            let drain = crate::stream::drain_value_stream(cx, &value, progress_token)?;
            session.record_stream_packets(drain.packets);
            Ok((McpToolResult::success(cx, value)?, drain.notifications))
        }
        Err(error) => Ok((McpToolResult::error(error.to_string()), Vec::new())),
    }
}

pub(crate) fn require_surface_capabilities(
    session: &McpSession,
    row: &McpSurfaceCard,
    gate_capability: CapabilityName,
) -> Result<()> {
    let required = required_capabilities_for_row(row, gate_capability);
    require_session_capabilities(session, &required)
}

fn resolve_tool_row(cx: &mut Cx, session: &McpSession, name: &str) -> Result<McpSurfaceCard> {
    project_mcp_surface(cx, &session.native_cards, &session.profile)?
        .into_iter()
        .find(|row| row.role == McpSurfaceRole::Tool && row.name == name)
        .ok_or_else(|| Error::Eval(format!("unknown MCP tool {name}")))
}

fn eval_request_for_row(
    row: &McpSurfaceCard,
    gate_capability: CapabilityName,
    arguments: Vec<Expr>,
    deadline_ms: Option<u64>,
    label: &'static str,
) -> Result<EvalRequest> {
    let symbol = row
        .symbol
        .clone()
        .ok_or_else(|| Error::Eval(format!("{label} {} has no callable symbol", row.name)))?;
    let required_capabilities = required_capabilities_for_row(row, gate_capability);
    Ok(EvalRequest {
        expr: Expr::Call {
            operator: Box::new(Expr::Symbol(symbol)),
            args: arguments,
        },
        result_shape: row.output_shape.clone(),
        required_capabilities,
        deadline: deadline_ms.map(Duration::from_millis),
        consistency: Consistency::LocalFirst,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    })
}

fn required_capabilities_for_row(
    row: &McpSurfaceCard,
    gate_capability: CapabilityName,
) -> Vec<CapabilityName> {
    let mut required_capabilities = vec![gate_capability];
    required_capabilities.extend(row.capabilities.clone());
    required_capabilities.sort();
    required_capabilities.dedup();
    required_capabilities
}

fn require_session_capabilities(session: &McpSession, required: &[CapabilityName]) -> Result<()> {
    for capability in required {
        if !session
            .granted_capabilities
            .iter()
            .any(|granted| granted == capability)
        {
            return Err(Error::CapabilityDenied {
                capability: capability.clone(),
            });
        }
    }
    Ok(())
}

fn validate_input(cx: &mut Cx, input_shape: Option<&ShapeRef>, values: &[Value]) -> Result<()> {
    let Some(shape) = input_shape else {
        return Ok(());
    };
    let args_value = cx.factory().list(values.to_vec())?;
    let matched = check_value_report(cx, shape, args_value)?;
    if matched.accepted {
        Ok(())
    } else {
        Err(Error::WrongShape {
            expected: shape_id(shape),
            diagnostics: matched.diagnostics,
        })
    }
}

fn call_local(
    cx: &mut Cx,
    row: &McpSurfaceCard,
    request: &EvalRequest,
    values: Vec<Value>,
) -> Result<Value> {
    let symbol = callable_symbol(&request.expr)?;
    let value = cx.call_function(symbol, Args::new(values))?;
    validate_output(cx, row.output_shape.as_ref(), value)
}

fn validate_output(cx: &mut Cx, output_shape: Option<&ShapeRef>, value: Value) -> Result<Value> {
    let Some(shape) = output_shape else {
        return Ok(value);
    };
    let matched = check_value_report(cx, shape, value.clone())?;
    if matched.accepted {
        Ok(value)
    } else {
        Err(Error::WrongShape {
            expected: shape_id(shape),
            diagnostics: matched.diagnostics,
        })
    }
}

fn callable_symbol(expr: &Expr) -> Result<&Symbol> {
    let Expr::Call { operator, .. } = expr else {
        return Err(Error::Eval("MCP tool request is not a call".to_owned()));
    };
    let Expr::Symbol(symbol) = operator.as_ref() else {
        return Err(Error::Eval(
            "MCP tool request operator is not a symbol".to_owned(),
        ));
    };
    Ok(symbol)
}

fn shape_id(shape: &ShapeRef) -> ShapeId {
    shape
        .object()
        .as_shape()
        .and_then(|shape| shape.id())
        .unwrap_or(ShapeId(0))
}
