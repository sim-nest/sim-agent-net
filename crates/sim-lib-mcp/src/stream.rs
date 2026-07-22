use sim_codec_mcp::{McpEnvelope, McpNotification};
use sim_kernel::{Cx, Error, Expr, NumberLiteral, Result, Symbol, Value};
use sim_lib_stream_core::{StreamDiagnostic, StreamPacket};

use crate::McpSession;

const MAX_PROGRESS_CHUNKS: usize = 1024;

#[derive(Default)]
pub(crate) struct McpStreamDrain {
    pub notifications: Vec<McpEnvelope>,
    pub packets: Vec<StreamPacket>,
}

/// Returns the stream data kind tagging MCP progress packets.
pub fn mcp_progress_data_kind() -> Symbol {
    Symbol::qualified("stream/data", "mcp-progress")
}

/// Returns the stream data kind tagging MCP cancellation packets.
pub fn mcp_cancelled_data_kind() -> Symbol {
    Symbol::qualified("stream/data", "mcp-cancelled")
}

/// Returns the diagnostic kind for a truncated MCP stream.
pub fn mcp_truncated_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/mcp", "Truncated")
}

/// Returns the diagnostic kind for a failed MCP stream.
pub fn mcp_failed_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/mcp", "Failed")
}

/// Returns the diagnostic kind for an overflowed MCP stream.
pub fn mcp_overflowed_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/mcp", "Overflowed")
}

pub(crate) fn progress_token_from_params(params: &Expr) -> Option<Expr> {
    let meta = map_field(params, "_meta")?;
    map_field(meta, "progressToken").cloned()
}

pub(crate) fn drain_value_stream(
    cx: &mut Cx,
    value: &Value,
    progress_token: Option<&Expr>,
) -> Result<McpStreamDrain> {
    let Some(stream) = value.object().as_stream() else {
        return Ok(McpStreamDrain::default());
    };
    let mut drain = McpStreamDrain::default();
    for index in 0..MAX_PROGRESS_CHUNKS {
        let item = match stream.next(cx) {
            Ok(Some(item)) => item,
            Ok(None) => return Ok(drain),
            Err(error) => {
                let packet = diagnostic_packet(mcp_failed_diagnostic_kind(), error.to_string());
                push_packet(&mut drain, progress_token, index + 1, packet);
                return Ok(drain);
            }
        };
        let packet = stream_value_to_packet(cx, item);
        push_packet(&mut drain, progress_token, index + 1, packet);
    }
    let packet = diagnostic_packet(
        mcp_truncated_diagnostic_kind(),
        format!("MCP progress stream exceeded {MAX_PROGRESS_CHUNKS} chunks"),
    );
    push_packet(&mut drain, progress_token, MAX_PROGRESS_CHUNKS + 1, packet);
    Ok(drain)
}

pub(crate) fn apply_cancel_notification(session: &mut McpSession, params: Expr) -> Result<bool> {
    let (request_id, reason) = cancellation_params(params)?;
    if !session.request_is_active(&request_id) {
        return Ok(false);
    }
    session.mark_request_cancelled(&request_id);
    session.record_stream_packet(cancellation_packet(request_id, reason));
    Ok(true)
}

pub(crate) fn cancellation_packet(request_id: Expr, reason: Option<String>) -> StreamPacket {
    StreamPacket::data(
        mcp_cancelled_data_kind(),
        Expr::Map(vec![
            field("requestId", request_id),
            field("reason", reason.map(Expr::String).unwrap_or(Expr::Nil)),
        ]),
    )
}

fn push_packet(
    drain: &mut McpStreamDrain,
    progress_token: Option<&Expr>,
    ordinal: usize,
    packet: StreamPacket,
) {
    if let Some(token) = progress_token {
        drain
            .notifications
            .push(progress_notification(token, ordinal, &packet));
    }
    drain.packets.push(packet);
}

fn progress_notification(token: &Expr, ordinal: usize, packet: &StreamPacket) -> McpEnvelope {
    McpEnvelope::Notification(McpNotification {
        method: "notifications/progress".to_owned(),
        params: Expr::Map(vec![
            field("progressToken", token.clone()),
            field("progress", ordinal_expr(ordinal)),
            field("message", Expr::String(packet_message(packet))),
            field("data", packet.to_expr()),
        ]),
    })
}

fn stream_value_to_packet(cx: &mut Cx, value: Value) -> StreamPacket {
    match value.object().as_expr(cx) {
        Ok(expr) => match StreamPacket::try_from(expr.clone()) {
            Ok(packet) => packet,
            Err(_) => StreamPacket::data(mcp_progress_data_kind(), expr),
        },
        Err(error) => diagnostic_packet(mcp_failed_diagnostic_kind(), error.to_string()),
    }
}

fn diagnostic_packet(kind: Symbol, message: String) -> StreamPacket {
    StreamPacket::Diagnostic(StreamDiagnostic::new(kind, message))
}

fn cancellation_params(params: Expr) -> Result<(Expr, Option<String>)> {
    let fields = match params {
        Expr::Map(fields) => fields,
        _ => {
            return Err(Error::TypeMismatch {
                expected: "MCP cancellation params map",
                found: "non-map",
            });
        }
    };
    let request_id = fields
        .iter()
        .find_map(|(key, value)| (key_name(key)? == "requestId").then_some(value.clone()))
        .ok_or_else(|| Error::Eval("MCP cancellation is missing requestId".to_owned()))?;
    let reason = fields.iter().find_map(|(key, value)| {
        (key_name(key)? == "reason").then(|| match value {
            Expr::String(reason) => Some(reason.clone()),
            Expr::Nil => None,
            other => Some(format!("{other:?}")),
        })?
    });
    Ok((request_id, reason))
}

fn packet_message(packet: &StreamPacket) -> String {
    match packet {
        StreamPacket::Data(packet) => packet.kind.to_string(),
        StreamPacket::Diagnostic(packet) => packet.message().to_owned(),
        StreamPacket::Pcm(_) => "stream/packet/pcm".to_owned(),
        StreamPacket::Midi(_) => "stream/packet/midi".to_owned(),
    }
}

use sim_value::access::field_any as map_field;

fn key_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Some(symbol.name.as_ref()),
        Expr::String(value) => Some(value.as_str()),
        _ => None,
    }
}

use sim_value::build::entry as field;

fn ordinal_expr(ordinal: usize) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "i64"),
        canonical: ordinal.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_key_names_accept_bare_symbol_or_string_only() {
        let cases = [
            (Expr::Symbol(Symbol::new("bare")), Some("bare")),
            (Expr::String("string".to_owned()), Some("string")),
            (Expr::Symbol(Symbol::qualified("mcp", "qualified")), None),
        ];

        for (expr, expected) in cases {
            assert_eq!(key_name(&expr), expected);
        }
    }
}
