use std::time::Duration;

use sim_citizen::value_from_expr;
use sim_kernel::{
    CapabilityName, Consistency, Cx, Diagnostic, Error, EvalMode, EvalReply, EvalRequest, Expr,
    ObjectCompat, ReadPolicy, Result, Severity, Symbol, Value,
};
use sim_value::capability_names_from_expr;

use crate::helpers::parse_optional_duration;
use crate::{FrameKind, ServerFrame};

/// Encodes an [`EvalRequest`] into a request [`ServerFrame`] under `codec`.
///
/// Carries the request's consistency, required capabilities, trace flag, and
/// deadline onto the frame envelope.
pub fn server_frame_from_request(
    cx: &mut Cx,
    codec: &Symbol,
    request: EvalRequest,
) -> Result<ServerFrame> {
    let expr = request.as_expr(cx)?;
    let mut frame = ServerFrame::from_expr(
        cx,
        codec.clone(),
        FrameKind::Request,
        &expr,
        request.consistency,
        request.required_capabilities.clone(),
        request.trace,
    )?;
    frame.envelope.deadline = request.deadline;
    Ok(frame)
}

/// Encodes an [`EvalReply`] into a response [`ServerFrame`] under `codec`.
///
/// Applies the given `consistency` and sets the envelope trace flag from the
/// reply's trace value.
pub fn server_frame_from_reply(
    cx: &mut Cx,
    codec: &Symbol,
    reply: EvalReply,
    consistency: Consistency,
) -> Result<ServerFrame> {
    let expr = reply.as_expr(cx)?;
    let mut frame = ServerFrame::from_expr(
        cx,
        codec.clone(),
        FrameKind::Response,
        &expr,
        consistency,
        Vec::new(),
        reply.trace.is_some(),
    )?;
    if let Some(trace) = reply.trace {
        frame.envelope.trace = !matches!(trace.object().as_expr(cx)?, Expr::Nil);
    }
    Ok(frame)
}

/// Decodes a request [`ServerFrame`] back into an [`EvalRequest`].
///
/// Returns an error when the frame is not a request frame.
pub fn eval_request_from_frame(cx: &mut Cx, frame: &ServerFrame) -> Result<EvalRequest> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "expected request frame, found {}",
            frame.kind.as_symbol()
        )));
    }
    let expr = frame.decode_expr(cx, ReadPolicy::default())?;
    eval_request_from_expr(cx, expr)
}

/// Decodes a response [`ServerFrame`] back into an [`EvalReply`].
///
/// Returns an error when the frame is not a response frame.
pub fn eval_reply_from_frame(cx: &mut Cx, frame: &ServerFrame) -> Result<EvalReply> {
    if frame.kind == FrameKind::Error {
        let detail = match frame.decode_expr(cx, ReadPolicy::default())? {
            Expr::String(detail) => detail,
            detail => format!("{detail:?}"),
        };
        return Err(Error::Eval(format!("remote evaluation failed: {detail}")));
    }
    if frame.kind != FrameKind::Response {
        return Err(Error::Eval(format!(
            "expected response frame, found {}",
            frame.kind.as_symbol()
        )));
    }
    let expr = frame.decode_expr(cx, ReadPolicy::default())?;
    eval_reply_from_expr(cx, expr)
}

fn eval_request_from_expr(cx: &mut Cx, expr: Expr) -> Result<EvalRequest> {
    let request_expr = required_table_field(&expr, "expr")?.clone();
    let result_shape = parse_result_shape_expr(cx, required_table_field(&expr, "result-shape")?)?;
    let required_capabilities = parse_capability_expr(required_table_field(&expr, "requires")?)?;
    let deadline = parse_deadline_expr(required_table_field(&expr, "deadline")?)?;
    let consistency = parse_consistency_expr(required_table_field(&expr, "consistency")?)?;
    let mode = optional_table_value(&expr, "mode")
        .map(parse_mode_expr)
        .transpose()?
        .unwrap_or(EvalMode::Eval);
    let answer_limit = optional_table_value(&expr, "answer-limit")
        .map(parse_optional_usize_expr)
        .transpose()?
        .flatten();
    let stream_buffer = optional_table_value(&expr, "stream-buffer")
        .map(parse_optional_usize_expr)
        .transpose()?
        .flatten();
    let stream = optional_table_value(&expr, "stream")
        .map(parse_bool_expr)
        .transpose()?
        .unwrap_or(false);
    let trace = parse_bool_expr(required_table_field(&expr, "trace")?)?;
    Ok(EvalRequest {
        expr: request_expr,
        result_shape,
        required_capabilities,
        deadline,
        consistency,
        mode,
        answer_limit,
        stream_buffer,
        stream,
        trace,
    })
}

fn eval_reply_from_expr(cx: &mut Cx, expr: Expr) -> Result<EvalReply> {
    let value = value_from_expr(cx, required_table_field(&expr, "value")?)?;
    let diagnostics = parse_diagnostics_expr(required_table_field(&expr, "diagnostics")?)?;
    let trace = parse_optional_value_expr(cx, required_table_field(&expr, "trace")?)?;
    Ok(EvalReply {
        value,
        diagnostics,
        trace,
    })
}

fn required_table_field<'a>(expr: &'a Expr, key: &str) -> Result<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return Err(Error::TypeMismatch {
            expected: "table expression",
            found: "non-table",
        });
    };
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match entry_key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(entry_value),
            _ => None,
        })
        .ok_or_else(|| Error::Eval(format!("missing frame field {key}")))
}

fn optional_table_value<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match entry_key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(entry_value),
            _ => None,
        })
}

fn parse_result_shape_expr(cx: &mut Cx, expr: &Expr) -> Result<Option<sim_kernel::ShapeRef>> {
    if matches!(expr, Expr::Nil) {
        return Ok(None);
    }
    if let Expr::Symbol(symbol) = expr {
        if let Ok(shape) = cx.resolve_shape(symbol) {
            return Ok(Some(shape));
        }
        if symbol.name.as_ref() == "instance-shape"
            && let Some(namespace) = &symbol.namespace
        {
            let class_symbol = parse_qualified_symbol(namespace);
            if let Ok(class_value) = cx.resolve_class(&class_symbol)
                && let Some(class) = class_value.object().as_class()
            {
                return Ok(Some(class.instance_shape(cx)?));
            }
        }
    }
    let value = cx.eval_expr(expr.clone())?;
    if let Some(class) = value.object().as_class() {
        return Ok(Some(class.instance_shape(cx)?));
    }
    Err(Error::TypeMismatch {
        expected: "shape or class",
        found: "non-shape",
    })
}

fn parse_qualified_symbol(text: &str) -> Symbol {
    match text.rsplit_once('/') {
        Some((namespace, name)) => Symbol::qualified(namespace.to_owned(), name.to_owned()),
        None => Symbol::new(text.to_owned()),
    }
}

fn parse_capability_expr(expr: &Expr) -> Result<Vec<CapabilityName>> {
    capability_names_from_expr(expr)
}

fn parse_deadline_expr(expr: &Expr) -> Result<Option<Duration>> {
    parse_optional_duration(expr)
}

fn parse_consistency_expr(expr: &Expr) -> Result<Consistency> {
    let name = match expr {
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        _ => {
            return Err(Error::TypeMismatch {
                expected: "consistency symbol or string",
                found: "non-consistency",
            });
        }
    };
    match name.as_str() {
        "local-only" => Ok(Consistency::LocalOnly),
        "local-first" => Ok(Consistency::LocalFirst),
        "remote-only" => Ok(Consistency::RemoteOnly),
        _ => Err(Error::Eval(format!(
            "unsupported realize consistency {name}"
        ))),
    }
}

fn parse_mode_expr(expr: &Expr) -> Result<EvalMode> {
    let name = match expr {
        Expr::Symbol(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        _ => {
            return Err(Error::TypeMismatch {
                expected: "mode symbol or string",
                found: "non-mode",
            });
        }
    };
    match name.as_str() {
        "eval" => Ok(EvalMode::Eval),
        "logic" => Ok(EvalMode::Logic),
        _ => Err(Error::Eval(format!("unsupported realize mode {name}"))),
    }
}

fn parse_optional_usize_expr(expr: &Expr) -> Result<Option<usize>> {
    match expr {
        Expr::Nil => Ok(None),
        Expr::Number(number) => number
            .canonical
            .parse::<usize>()
            .map(Some)
            .map_err(|_| Error::Eval(format!("expected usize, found {}", number.canonical))),
        Expr::String(text) => text
            .parse::<usize>()
            .map(Some)
            .map_err(|_| Error::Eval(format!("expected usize, found {text}"))),
        _ => Err(Error::TypeMismatch {
            expected: "usize or nil",
            found: "non-usize",
        }),
    }
}

fn parse_bool_expr(expr: &Expr) -> Result<bool> {
    match expr {
        Expr::Bool(value) => Ok(*value),
        _ => Err(Error::TypeMismatch {
            expected: "bool",
            found: "non-bool",
        }),
    }
}

fn parse_diagnostics_expr(expr: &Expr) -> Result<Vec<Diagnostic>> {
    match expr {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => {
            items.iter().map(parse_diagnostic_expr).collect()
        }
        _ => Err(Error::TypeMismatch {
            expected: "diagnostic list",
            found: "non-list",
        }),
    }
}

fn parse_diagnostic_expr(expr: &Expr) -> Result<Diagnostic> {
    let severity = match required_table_field(expr, "severity")? {
        Expr::Symbol(symbol) if symbol.name.as_ref() == "error" => Severity::Error,
        Expr::Symbol(symbol) if symbol.name.as_ref() == "warning" => Severity::Warning,
        Expr::Symbol(symbol) if symbol.name.as_ref() == "info" => Severity::Info,
        Expr::Symbol(symbol) if symbol.name.as_ref() == "note" => Severity::Note,
        _ => {
            return Err(Error::TypeMismatch {
                expected: "diagnostic severity symbol",
                found: "non-severity",
            });
        }
    };
    let message = match required_table_field(expr, "message")? {
        Expr::String(text) => text.clone(),
        _ => {
            return Err(Error::TypeMismatch {
                expected: "diagnostic message string",
                found: "non-string",
            });
        }
    };
    let code = match required_table_field(expr, "code")? {
        Expr::Nil => None,
        Expr::Symbol(symbol) => Some(symbol.clone()),
        _ => {
            return Err(Error::TypeMismatch {
                expected: "diagnostic code symbol",
                found: "non-symbol",
            });
        }
    };
    let related = parse_diagnostics_expr(required_table_field(expr, "related")?)?;
    Ok(Diagnostic {
        severity,
        message,
        source: None,
        span: None,
        code,
        related,
    })
}

fn parse_optional_value_expr(cx: &mut Cx, expr: &Expr) -> Result<Option<Value>> {
    if matches!(expr, Expr::Nil) {
        return Ok(None);
    }
    value_from_expr(cx, expr).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_error_frames_preserve_the_server_diagnostic() {
        let mut cx = crate::tests::cx();
        let frame = ServerFrame::from_expr(
            &mut cx,
            Symbol::qualified("codec", "binary"),
            FrameKind::Error,
            &Expr::String("unknown symbol shared".to_owned()),
            Consistency::RemoteOnly,
            Vec::new(),
            false,
        )
        .unwrap();

        assert!(matches!(
            eval_reply_from_frame(&mut cx, &frame),
            Err(Error::Eval(message))
                if message == "remote evaluation failed: unknown symbol shared"
        ));
    }
}
