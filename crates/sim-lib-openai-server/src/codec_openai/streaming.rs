use serde_json::{Value, json};
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_stream_core::StreamPacket;

use crate::{codec_openai::encode_openai_responses_response, objects::GatewayEvent};

/// Selects which OpenAI server-sent-event surface to render gateway events as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenAiSseSurface {
    /// The OpenAI Responses API streaming surface.
    Responses,
    /// The OpenAI Chat Completions streaming surface.
    Chat,
}

/// Decoded gateway event payload: sequence number, event kind, and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayEventData {
    sequence: u64,
    kind: Symbol,
    payload: Expr,
}

impl GatewayEventData {
    /// Extracts the streaming-relevant fields from a [`GatewayEvent`].
    pub fn from_event(event: &GatewayEvent) -> Self {
        Self {
            sequence: event.sequence(),
            kind: event.kind().clone(),
            payload: event.payload().clone(),
        }
    }

    /// Decodes gateway event data from a stream data packet, erroring if the
    /// packet is not a data packet of the gateway event kind.
    pub fn from_packet(packet: &StreamPacket) -> Result<Self> {
        let StreamPacket::Data(data) = packet else {
            return Err(Error::TypeMismatch {
                expected: "OpenAI gateway data packet",
                found: "non-data packet",
            });
        };
        if data.kind != gateway_event_data_kind() {
            return Err(Error::Eval(format!(
                "expected OpenAI gateway data kind {}, found {}",
                gateway_event_data_kind(),
                data.kind
            )));
        }
        Ok(Self {
            sequence: string_field(&data.payload, "sequence")?
                .parse::<u64>()
                .map_err(|err| Error::Eval(format!("invalid gateway event sequence: {err}")))?,
            kind: symbol_value_field(&data.payload, "event-kind")?,
            payload: map_field(&data.payload, "payload")?.clone(),
        })
    }

    /// Returns the event sequence number.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the event kind symbol.
    pub fn kind(&self) -> &Symbol {
        &self.kind
    }

    /// Returns the event payload expression.
    pub fn payload(&self) -> &Expr {
        &self.payload
    }
}

/// Returns the stream data kind symbol `stream/data:openai-gateway-event`.
pub fn gateway_event_data_kind() -> Symbol {
    Symbol::qualified("stream/data", "openai-gateway-event")
}

/// Wraps gateway events as stream data packets of the gateway event kind.
pub fn gateway_event_data_packets(events: &[GatewayEvent]) -> Vec<StreamPacket> {
    events
        .iter()
        .map(|event| StreamPacket::data(gateway_event_data_kind(), event.to_expr()))
        .collect()
}

/// Decodes [`GatewayEventData`] from a stream data packet.
pub fn gateway_event_data_from_packet(packet: &StreamPacket) -> Result<GatewayEventData> {
    GatewayEventData::from_packet(packet)
}

/// Accumulates server-sent-event (SSE) framed bytes for a streamed response.
#[derive(Clone, Debug, Default)]
pub struct StreamSink {
    body: Vec<u8>,
    done: bool,
}

impl StreamSink {
    /// Creates an empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one `data:` SSE frame carrying the JSON `value`.
    pub fn event(&mut self, value: Value) -> Result<()> {
        let line = crate::objects::canonical_json_bytes(value);
        self.body.extend_from_slice(b"data: ");
        self.body.extend_from_slice(&line);
        self.body.extend_from_slice(b"\n\n");
        Ok(())
    }

    /// Appends the terminating `data: [DONE]` frame once, if not already done.
    pub fn done(&mut self) {
        if !self.done {
            self.body.extend_from_slice(b"data: [DONE]\n\n");
            self.done = true;
        }
    }

    /// Finalizes the stream (emitting `[DONE]`) and returns the framed bytes.
    pub fn into_bytes(mut self) -> Vec<u8> {
        self.done();
        self.body
    }
}

/// Encodes a sequence of gateway events as SSE bytes for the given surface,
/// using `response_id` and `created_at_ms` to stamp the emitted chunks.
pub fn encode_gateway_events_sse(
    events: &[GatewayEvent],
    surface: OpenAiSseSurface,
    response_id: &str,
    created_at_ms: u64,
) -> Result<Vec<u8>> {
    let packets = gateway_event_data_packets(events);
    let events = packets
        .iter()
        .map(gateway_event_data_from_packet)
        .collect::<Result<Vec<_>>>()?;
    let model = model_from_event_data(&events).unwrap_or_else(|| "fixture/echo".to_owned());
    let mut sink = StreamSink::new();
    match surface {
        OpenAiSseSurface::Responses => {
            for event in &events {
                if let Some(chunk) = responses_chunk(event, response_id, created_at_ms, &model)? {
                    sink.event(chunk)?;
                }
            }
        }
        OpenAiSseSurface::Chat => {
            for event in &events {
                if let Some(chunk) = chat_chunk(event, response_id, created_at_ms, &model)? {
                    sink.event(chunk)?;
                }
            }
        }
    }
    Ok(sink.into_bytes())
}

fn responses_chunk(
    event: &GatewayEventData,
    response_id: &str,
    created_at_ms: u64,
    model: &str,
) -> Result<Option<Value>> {
    Ok(match event.kind().name.as_ref() {
        "request-start" => Some(json!({
            "type": "response.created",
            "response": response_stub(response_id, created_at_ms, model, "created"),
        })),
        "plan-start" => Some(json!({
            "type": "response.metadata",
            "sequence": event.sequence(),
        })),
        "model-start" => Some(json!({
            "type": "response.in_progress",
            "response": response_stub(response_id, created_at_ms, model, "in_progress"),
        })),
        "delta" => Some(json!({
            "type": "response.output_text.delta",
            "delta": string_payload(event.payload())?,
        })),
        "usage" => Some(json!({
            "type": "response.usage",
            "usage": usage_json(event.payload())?,
        })),
        "error" => Some(json!({
            "type": "error",
            "error": error_json(event.payload()),
        })),
        "final" => Some(json!({
            "type": "response.completed",
            "response": final_response_json(event.payload(), response_id, created_at_ms)?,
        })),
        _ => None,
    })
}

fn chat_chunk(
    event: &GatewayEventData,
    response_id: &str,
    created_at_ms: u64,
    model: &str,
) -> Result<Option<Value>> {
    Ok(match event.kind().name.as_ref() {
        "model-start" => Some(chat_choice_chunk(
            response_id,
            created_at_ms,
            model,
            json!({"role": "assistant"}),
            Value::Null,
        )),
        "delta" => Some(chat_choice_chunk(
            response_id,
            created_at_ms,
            model,
            json!({"content": string_payload(event.payload())?}),
            Value::Null,
        )),
        "usage" => Some(json!({
            "id": response_id,
            "object": "chat.completion.chunk",
            "created": created_at_ms / 1000,
            "model": model,
            "choices": [],
            "usage": usage_json(event.payload())?,
        })),
        "error" => Some(json!({
            "type": "error",
            "error": error_json(event.payload()),
        })),
        "final" => Some(chat_choice_chunk(
            response_id,
            created_at_ms,
            model,
            json!({}),
            json!(finish_reason(event.payload()).unwrap_or_else(|_| "stop".to_owned())),
        )),
        _ => None,
    })
}

fn response_stub(response_id: &str, created_at_ms: u64, model: &str, status: &str) -> Value {
    json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at_ms / 1000,
        "status": status,
        "model": model,
    })
}

fn chat_choice_chunk(
    response_id: &str,
    created_at_ms: u64,
    model: &str,
    delta: Value,
    finish_reason: Value,
) -> Value {
    json!({
        "id": response_id,
        "object": "chat.completion.chunk",
        "created": created_at_ms / 1000,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason,
        }],
    })
}

fn final_response_json(expr: &Expr, response_id: &str, created_at_ms: u64) -> Result<Value> {
    let bytes = encode_openai_responses_response(expr, response_id, created_at_ms)?;
    serde_json::from_slice(&bytes).map_err(|err| {
        Error::Eval(format!(
            "openai codec failed to decode final response chunk: {err}"
        ))
    })
}

fn model_from_event_data(events: &[GatewayEventData]) -> Option<String> {
    events.iter().find_map(|event| {
        if event.kind().name.as_ref() == "model-start" {
            string_payload(event.payload()).ok()
        } else if event.kind().name.as_ref() == "final" {
            string_field(event.payload(), "model").ok()
        } else {
            None
        }
    })
}

fn finish_reason(expr: &Expr) -> Result<String> {
    symbol_field(expr, "stop-reason")
}

fn usage_json(expr: &Expr) -> Result<Value> {
    let Expr::Map(fields) = expr else {
        return Err(Error::Eval(
            "openai SSE usage payload must be a map".to_owned(),
        ));
    };
    let prompt = optional_u64_field(fields, "input-tokens")?.unwrap_or(0);
    let completion = optional_u64_field(fields, "output-tokens")?.unwrap_or(0);
    let total = optional_u64_field(fields, "total-tokens")?.unwrap_or(prompt + completion);
    Ok(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    }))
}

fn error_json(expr: &Expr) -> Value {
    match expr {
        Expr::String(message) => json!({"message": message}),
        other => json!({"message": format!("{other:?}")}),
    }
}

fn string_payload(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        other => Err(Error::Eval(format!(
            "openai SSE event payload must be a string, found {other:?}"
        ))),
    }
}

fn string_field(expr: &Expr, key: &str) -> Result<String> {
    match map_field(expr, key)? {
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::Eval(format!(
            "openai SSE field {key} must be a string"
        ))),
    }
}

fn symbol_field(expr: &Expr, key: &str) -> Result<String> {
    match map_field(expr, key)? {
        Expr::Symbol(symbol) => Ok(symbol.name.as_ref().to_owned()),
        _ => Err(Error::Eval(format!(
            "openai SSE field {key} must be a symbol"
        ))),
    }
}

fn symbol_value_field(expr: &Expr, key: &str) -> Result<Symbol> {
    match map_field(expr, key)? {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        _ => Err(Error::Eval(format!(
            "openai SSE field {key} must be a symbol"
        ))),
    }
}

fn map_field<'a>(expr: &'a Expr, key: &str) -> Result<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval("openai SSE payload must be a map".to_owned()));
    };
    sim_value::access::entry_field(entries, key)
        .ok_or_else(|| Error::Eval(format!("openai SSE payload missing {key}")))
}

fn optional_u64_field(entries: &[(Expr, Expr)], key: &str) -> Result<Option<u64>> {
    let Some(value) = entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    }) else {
        return Ok(None);
    };
    match value {
        Expr::Number(number) => number
            .canonical
            .parse::<u64>()
            .map(Some)
            .map_err(|err| Error::Eval(format!("openai SSE invalid {key}: {err}"))),
        Expr::String(text) => text
            .parse::<u64>()
            .map(Some)
            .map_err(|err| Error::Eval(format!("openai SSE invalid {key}: {err}"))),
        _ => Err(Error::Eval(format!(
            "openai SSE field {key} must be a number"
        ))),
    }
}
