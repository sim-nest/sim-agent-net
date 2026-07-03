use std::time::Duration;

use sim_kernel::{CapabilityName, Error, Expr, Result, Symbol};

use crate::{FrameEnvelope, FrameKind, LifecycleCommand, ServerAddress, ServerFrame};

pub(crate) fn endpoint_key(address: &ServerAddress) -> String {
    match address {
        ServerAddress::Local => "local".to_owned(),
        ServerAddress::Any => "any".to_owned(),
        ServerAddress::InProcess { thread } => format!("in-process:{thread}"),
        ServerAddress::Coroutine { id } => format!("coroutine:{id}"),
        ServerAddress::Tcp { host, port } => format!("tcp:{host}:{port}"),
        ServerAddress::Unix { path } => format!("unix:{}", path.display()),
        ServerAddress::Wasm { region } => format!("wasm:{region}"),
        ServerAddress::Http { url } => format!("http:{url}"),
        ServerAddress::Ws { url } => format!("ws:{url}"),
        ServerAddress::Sse { url } => format!("sse:{url}"),
        ServerAddress::Smtp { address } => format!("smtp:{address}"),
        ServerAddress::Imap { address, mailbox } => format!("imap:{address}:{mailbox}"),
        ServerAddress::Telegram { chat_id, bot } => format!("telegram:{chat_id}:{bot}"),
        ServerAddress::Matrix { room_id } => format!("matrix:{room_id}"),
        ServerAddress::Stdin => "stdin".to_owned(),
        ServerAddress::FileTail { path } => format!("file-tail:{}", path.display()),
        ServerAddress::Cron { spec } => format!("cron:{spec}"),
        ServerAddress::Webhook { route } => format!("webhook:{route}"),
        ServerAddress::Agent { agent } => format!("agent:{agent}"),
        ServerAddress::Pipeline { steps } => format!("pipeline:{}", steps.len()),
    }
}

pub(crate) fn frame_header_expr(frame: &ServerFrame) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("version")),
            Expr::String(frame.version.to_string()),
        ),
        (
            Expr::Symbol(Symbol::new("codec")),
            Expr::Symbol(frame.codec.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("msg-id")),
            optional_u64_expr(frame.msg_id),
        ),
        (
            Expr::Symbol(Symbol::new("correlate")),
            optional_u64_expr(frame.correlate),
        ),
        (
            Expr::Symbol(Symbol::new("kind")),
            frame_kind_expr(&frame.kind),
        ),
        (
            Expr::Symbol(Symbol::new("envelope")),
            envelope_expr(&frame.envelope),
        ),
    ])
}

pub(crate) fn frame_from_header_expr(expr: &Expr, payload: Vec<u8>) -> Result<ServerFrame> {
    Ok(ServerFrame {
        version: required_u16_field(expr, "version")?,
        codec: required_symbol_field(expr, "codec")?,
        msg_id: optional_u64_field(expr, "msg-id")?,
        correlate: optional_u64_field(expr, "correlate")?,
        kind: parse_frame_kind(required_field(expr, "kind")?)?,
        envelope: parse_envelope(required_field(expr, "envelope")?)?,
        payload,
    })
}

fn envelope_expr(envelope: &FrameEnvelope) -> Expr {
    Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("deadline-ms")),
            match envelope.deadline {
                Some(duration) => Expr::String(duration.as_millis().to_string()),
                None => Expr::Nil,
            },
        ),
        (
            Expr::Symbol(Symbol::new("consistency")),
            Expr::Symbol(envelope.consistency.as_symbol()),
        ),
        (
            Expr::Symbol(Symbol::new("trace")),
            Expr::Bool(envelope.trace),
        ),
        (
            Expr::Symbol(Symbol::new("requires")),
            Expr::List(
                envelope
                    .required_capabilities
                    .iter()
                    .map(|capability| Expr::String(capability.as_str().to_owned()))
                    .collect(),
            ),
        ),
        (
            Expr::Symbol(Symbol::new("reply-codec-hint")),
            match &envelope.reply_codec_hint {
                Some(codec) => Expr::Symbol(codec.clone()),
                None => Expr::Nil,
            },
        ),
        (
            Expr::Symbol(Symbol::new("role")),
            match &envelope.role {
                Some(role) => Expr::Symbol(role.clone()),
                None => Expr::Nil,
            },
        ),
        (
            Expr::Symbol(Symbol::new("hop")),
            Expr::String(envelope.hop.to_string()),
        ),
        (
            Expr::Symbol(Symbol::new("trigger-source")),
            match &envelope.trigger_source {
                Some(source) => Expr::Symbol(source.clone()),
                None => Expr::Nil,
            },
        ),
    ])
}

fn parse_envelope(expr: &Expr) -> Result<FrameEnvelope> {
    Ok(FrameEnvelope {
        deadline: optional_u64_field(expr, "deadline-ms")?.map(Duration::from_millis),
        consistency: parse_consistency(required_field(expr, "consistency")?)?,
        trace: required_bool_field(expr, "trace")?,
        required_capabilities: parse_capabilities(required_field(expr, "requires")?)?,
        reply_codec_hint: optional_symbol_field(expr, "reply-codec-hint")?,
        role: optional_symbol_field(expr, "role")?,
        hop: required_u32_field(expr, "hop")?,
        trigger_source: optional_symbol_field(expr, "trigger-source")?,
    })
}

fn frame_kind_expr(kind: &FrameKind) -> Expr {
    match kind {
        FrameKind::Request => Expr::Symbol(Symbol::new("request")),
        FrameKind::Response => Expr::Symbol(Symbol::new("response")),
        FrameKind::Error => Expr::Symbol(Symbol::new("error")),
        FrameKind::Notify => Expr::Symbol(Symbol::new("notify")),
        FrameKind::StreamStart => Expr::Symbol(Symbol::new("stream-start")),
        FrameKind::StreamChunk => Expr::Symbol(Symbol::new("stream-chunk")),
        FrameKind::StreamEnd => Expr::Symbol(Symbol::new("stream-end")),
        FrameKind::Negotiate { codecs } => Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(Symbol::new("negotiate")),
            ),
            (
                Expr::Symbol(Symbol::new("codecs")),
                Expr::List(codecs.iter().cloned().map(Expr::Symbol).collect()),
            ),
        ]),
        FrameKind::Ping => Expr::Symbol(Symbol::new("ping")),
        FrameKind::Pong => Expr::Symbol(Symbol::new("pong")),
        FrameKind::Lifecycle { command } => Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(Symbol::new("lifecycle")),
            ),
            (
                Expr::Symbol(Symbol::new("command")),
                Expr::Symbol(match command {
                    LifecycleCommand::Start => Symbol::new("start"),
                    LifecycleCommand::Stop => Symbol::new("stop"),
                    LifecycleCommand::Suspend => Symbol::new("suspend"),
                    LifecycleCommand::Resume => Symbol::new("resume"),
                    LifecycleCommand::Health => Symbol::new("health"),
                }),
            ),
        ]),
        FrameKind::Trigger { source, when_ms } => Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(Symbol::new("trigger")),
            ),
            (
                Expr::Symbol(Symbol::new("source")),
                Expr::Symbol(source.clone()),
            ),
            (
                Expr::Symbol(Symbol::new("when-ms")),
                Expr::String(when_ms.to_string()),
            ),
        ]),
        FrameKind::Role { role, hop } => Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("kind")),
                Expr::Symbol(Symbol::new("role")),
            ),
            (
                Expr::Symbol(Symbol::new("role")),
                Expr::Symbol(role.clone()),
            ),
            (
                Expr::Symbol(Symbol::new("hop")),
                Expr::String(hop.to_string()),
            ),
        ]),
    }
}

fn parse_frame_kind(expr: &Expr) -> Result<FrameKind> {
    match expr {
        Expr::Symbol(symbol) => match symbol.name.as_ref() {
            "request" => Ok(FrameKind::Request),
            "response" => Ok(FrameKind::Response),
            "error" => Ok(FrameKind::Error),
            "notify" => Ok(FrameKind::Notify),
            "stream-start" => Ok(FrameKind::StreamStart),
            "stream-chunk" => Ok(FrameKind::StreamChunk),
            "stream-end" => Ok(FrameKind::StreamEnd),
            "ping" => Ok(FrameKind::Ping),
            "pong" => Ok(FrameKind::Pong),
            other => Err(Error::Eval(format!("unsupported frame kind {other}"))),
        },
        Expr::Map(_) => {
            let kind = required_symbol_field(expr, "kind")?;
            match kind.name.as_ref() {
                "negotiate" => Ok(FrameKind::Negotiate {
                    codecs: parse_symbol_list(required_field(expr, "codecs")?)?,
                }),
                "lifecycle" => Ok(FrameKind::Lifecycle {
                    command: match required_symbol_field(expr, "command")?.name.as_ref() {
                        "start" => LifecycleCommand::Start,
                        "stop" => LifecycleCommand::Stop,
                        "suspend" => LifecycleCommand::Suspend,
                        "resume" => LifecycleCommand::Resume,
                        "health" => LifecycleCommand::Health,
                        other => {
                            return Err(Error::Eval(format!(
                                "unsupported lifecycle command {other}"
                            )));
                        }
                    },
                }),
                "trigger" => Ok(FrameKind::Trigger {
                    source: required_symbol_field(expr, "source")?,
                    when_ms: required_u64_field(expr, "when-ms")?,
                }),
                "role" => Ok(FrameKind::Role {
                    role: required_symbol_field(expr, "role")?,
                    hop: required_u32_field(expr, "hop")?,
                }),
                other => Err(Error::Eval(format!("unsupported frame kind {other}"))),
            }
        }
        _ => Err(Error::TypeMismatch {
            expected: "frame kind expression",
            found: "non-kind",
        }),
    }
}

fn parse_consistency(expr: &Expr) -> Result<sim_kernel::Consistency> {
    let Expr::Symbol(symbol) = expr else {
        return Err(Error::TypeMismatch {
            expected: "consistency symbol",
            found: "non-symbol",
        });
    };
    match symbol.name.as_ref() {
        "local-first" => Ok(sim_kernel::Consistency::LocalFirst),
        "remote-only" => Ok(sim_kernel::Consistency::RemoteOnly),
        "local-only" => Ok(sim_kernel::Consistency::LocalOnly),
        other => Err(Error::Eval(format!("unsupported consistency {other}"))),
    }
}

fn parse_capabilities(expr: &Expr) -> Result<Vec<CapabilityName>> {
    let (Expr::List(items) | Expr::Vector(items)) = expr else {
        return Err(Error::TypeMismatch {
            expected: "capability list",
            found: "non-list",
        });
    };
    items
        .iter()
        .map(|item| match item {
            Expr::String(text) => Ok(CapabilityName::new(text.clone())),
            Expr::Symbol(symbol) => Ok(CapabilityName::new(symbol.to_string())),
            _ => Err(Error::TypeMismatch {
                expected: "capability name",
                found: "non-name",
            }),
        })
        .collect()
}

fn parse_symbol_list(expr: &Expr) -> Result<Vec<Symbol>> {
    let (Expr::List(items) | Expr::Vector(items)) = expr else {
        return Err(Error::TypeMismatch {
            expected: "symbol list",
            found: "non-list",
        });
    };
    items
        .iter()
        .map(|item| match item {
            Expr::Symbol(symbol) => Ok(symbol.clone()),
            _ => Err(Error::TypeMismatch {
                expected: "symbol",
                found: "non-symbol",
            }),
        })
        .collect()
}

fn required_field<'a>(expr: &'a Expr, key: &str) -> Result<&'a Expr> {
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
        .ok_or_else(|| Error::Eval(format!("missing transport field {key}")))
}

fn required_symbol_field(expr: &Expr, key: &str) -> Result<Symbol> {
    match required_field(expr, key)? {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        _ => Err(Error::TypeMismatch {
            expected: "symbol",
            found: "non-symbol",
        }),
    }
}

fn optional_symbol_field(expr: &Expr, key: &str) -> Result<Option<Symbol>> {
    match required_field(expr, key)? {
        Expr::Nil => Ok(None),
        Expr::Symbol(symbol) => Ok(Some(symbol.clone())),
        _ => Err(Error::TypeMismatch {
            expected: "symbol or nil",
            found: "non-symbol",
        }),
    }
}

fn required_bool_field(expr: &Expr, key: &str) -> Result<bool> {
    match required_field(expr, key)? {
        Expr::Bool(value) => Ok(*value),
        _ => Err(Error::TypeMismatch {
            expected: "bool",
            found: "non-bool",
        }),
    }
}

fn required_u16_field(expr: &Expr, key: &str) -> Result<u16> {
    parse_integer_field(expr, key)?
        .try_into()
        .map_err(|_| Error::Eval(format!("transport field {key} exceeds u16")))
}

fn required_u32_field(expr: &Expr, key: &str) -> Result<u32> {
    parse_integer_field(expr, key)?
        .try_into()
        .map_err(|_| Error::Eval(format!("transport field {key} exceeds u32")))
}

fn required_u64_field(expr: &Expr, key: &str) -> Result<u64> {
    parse_integer_field(expr, key)
}

fn optional_u64_field(expr: &Expr, key: &str) -> Result<Option<u64>> {
    match required_field(expr, key)? {
        Expr::Nil => Ok(None),
        Expr::String(text) => {
            Ok(Some(text.parse().map_err(|_| {
                Error::Eval(format!("transport field {key} must be an integer"))
            })?))
        }
        _ => Err(Error::TypeMismatch {
            expected: "integer string or nil",
            found: "non-integer",
        }),
    }
}

fn parse_integer_field(expr: &Expr, key: &str) -> Result<u64> {
    match required_field(expr, key)? {
        Expr::String(text) => text
            .parse()
            .map_err(|_| Error::Eval(format!("transport field {key} must be an integer"))),
        _ => Err(Error::TypeMismatch {
            expected: "integer string",
            found: "non-string",
        }),
    }
}

fn optional_u64_expr(value: Option<u64>) -> Expr {
    value
        .map(|value| Expr::String(value.to_string()))
        .unwrap_or(Expr::Nil)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_frame_kinds_round_trip_through_header() {
        for (kind, symbol) in [
            (FrameKind::StreamStart, "stream-start"),
            (FrameKind::StreamChunk, "stream-chunk"),
            (FrameKind::StreamEnd, "stream-end"),
        ] {
            let frame = ServerFrame::new(
                Symbol::qualified("codec", "lisp"),
                kind.clone(),
                FrameEnvelope::default(),
                b"payload".to_vec(),
            );
            let header = frame_header_expr(&frame);
            let decoded = frame_from_header_expr(&header, frame.payload.clone()).unwrap();

            assert_eq!(decoded.kind, kind);
            assert_eq!(decoded.kind.as_symbol(), Symbol::new(symbol));
            assert_eq!(decoded.payload, b"payload".to_vec());
        }
    }
}
