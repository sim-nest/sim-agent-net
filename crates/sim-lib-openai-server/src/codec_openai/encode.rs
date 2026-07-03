use serde_json::{Value, json};
use sim_codec_chat::{is_model_request_expr, validate_chat_transcript};
use sim_codec_json::json_number_to_u64;
use sim_kernel::{CodecId, Error, Expr, Result};

use crate::codec_openai::codec_error;
use crate::codec_openai::decode::map_field;
use crate::codec_openai::shapes::OpenAiCodecOptions;

/// Encodes a model-request transcript into OpenAI chat-completion request JSON,
/// applying the model, streaming, and tools settings from `options`.
pub fn encode_openai_request(expr: &Expr, options: &OpenAiCodecOptions) -> Result<Vec<u8>> {
    if !is_model_request_expr(expr) {
        return Err(Error::Eval(
            "openai codec expects a model-request transcript".to_owned(),
        ));
    }
    validate_chat_transcript(expr)?;
    let mut payload = json!({
        "model": options.model,
        "stream": options.stream,
        "messages": transcript_messages(expr)?,
        "tools": if options.tools { Value::Array(Vec::new()) } else { Value::Null },
    });
    if options.stream
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("stream_options".to_owned(), json!({"include_usage": true}));
    }
    serde_json::to_vec(&payload)
        .map_err(|err| Error::Eval(format!("openai codec failed to encode request: {err}")))
}

/// Encodes a response transcript into OpenAI chat-completion response JSON.
pub fn encode_openai_response(expr: &Expr) -> Result<Vec<u8>> {
    let value = response_json(expr)?;
    serde_json::to_vec(&value)
        .map_err(|err| Error::Eval(format!("openai codec failed to encode response: {err}")))
}

/// Encodes a response transcript into OpenAI Responses API response JSON,
/// stamping it with `response_id` and `created_at_ms`.
pub fn encode_openai_responses_response(
    expr: &Expr,
    response_id: &str,
    created_at_ms: u64,
) -> Result<Vec<u8>> {
    let value = responses_response_json(expr, response_id, created_at_ms)?;
    serde_json::to_vec(&value).map_err(|err| {
        Error::Eval(format!(
            "openai codec failed to encode responses response: {err}"
        ))
    })
}

pub(crate) fn encode_openai_response_for_codec(codec: CodecId, expr: &Expr) -> Result<String> {
    if !marker_is_true(expr, "model-response") {
        return Err(codec_error(
            codec,
            "openai codec expects a model-response transcript",
        ));
    }
    validate_chat_transcript(expr).map_err(|err| match err {
        Error::Eval(message) => codec_error(codec, message),
        other => other,
    })?;
    let value = response_json(expr).map_err(|err| match err {
        Error::Eval(message) => codec_error(codec, message),
        other => other,
    })?;
    serde_json::to_string(&value).map_err(|err| codec_error(codec, err))
}

fn response_json(expr: &Expr) -> Result<Value> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval(
            "openai codec expects response transcript as a map".to_owned(),
        ));
    };
    let model = string_field(entries, "model")?;
    let finish_reason = symbol_field(entries, "stop-reason")?;
    Ok(json!({
        "id": "chatcmpl-sim",
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": response_text(entries)?,
            },
            "finish_reason": finish_reason,
        }],
        "usage": response_usage(entries)?,
    }))
}

fn responses_response_json(expr: &Expr, response_id: &str, created_at_ms: u64) -> Result<Value> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval(
            "openai codec expects response transcript as a map".to_owned(),
        ));
    };
    let model = string_field(entries, "model")?;
    let output_text = response_text(entries)?;
    Ok(json!({
        "id": response_id,
        "object": "response",
        "created_at": created_at_ms / 1000,
        "status": "completed",
        "model": model,
        "output": [{
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": output_text,
                "annotations": [],
            }],
        }],
        "output_text": output_text,
        "usage": response_usage(entries)?,
    }))
}

fn response_text(entries: &[(Expr, Expr)]) -> Result<String> {
    list_field(map_field(entries, "content")?)?
        .iter()
        .map(text_content)
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join(""))
}

fn response_usage(entries: &[(Expr, Expr)]) -> Result<Value> {
    let Some(usage) = entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == "usage" => Some(value),
        _ => None,
    }) else {
        return Ok(Value::Null);
    };
    let Expr::Map(fields) = usage else {
        return Err(Error::Eval(
            "openai codec usage field must be a map".to_owned(),
        ));
    };
    let prompt = optional_u64_field(fields, "input-tokens")?;
    let completion = optional_u64_field(fields, "output-tokens")?;
    let total =
        optional_u64_field(fields, "total-tokens")?.or_else(|| match (prompt, completion) {
            (Some(left), Some(right)) => Some(left + right),
            _ => None,
        });
    Ok(json!({
        "prompt_tokens": prompt.unwrap_or(0),
        "completion_tokens": completion.unwrap_or(0),
        "total_tokens": total.unwrap_or(0),
    }))
}

fn transcript_messages(expr: &Expr) -> Result<Vec<Value>> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval(
            "openai codec expects request transcript as a map".to_owned(),
        ));
    };
    let mut messages = list_field(map_field(entries, "messages")?)?
        .iter()
        .map(message_to_json)
        .collect::<Result<Vec<_>>>()?;
    messages.push(json!({
        "role": "user",
        "content": [{
            "type": "text",
            "text": flatten_expr(map_field(entries, "task")?),
        }],
    }));
    Ok(messages)
}

fn message_to_json(expr: &Expr) -> Result<Value> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval("openai codec message must be a map".to_owned()));
    };
    Ok(json!({
        "role": symbol_field(entries, "role")?,
        "content": list_field(map_field(entries, "content")?)?
            .iter()
            .map(content_part_to_json)
            .collect::<Result<Vec<_>>>()?,
    }))
}

fn content_part_to_json(expr: &Expr) -> Result<Value> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval(
            "openai codec content part must be a map".to_owned(),
        ));
    };
    match symbol_field(entries, "type")?.as_str() {
        "text" => Ok(json!({
            "type": "text",
            "text": string_field(entries, "text")?,
        })),
        other => Err(Error::Eval(format!(
            "openai codec does not support content part type {other}"
        ))),
    }
}

fn text_content(expr: &Expr) -> Result<String> {
    let Expr::Map(entries) = expr else {
        return Err(Error::Eval(
            "openai codec content part must be a map".to_owned(),
        ));
    };
    match symbol_field(entries, "type")?.as_str() {
        "text" => string_field(entries, "text"),
        other => Err(Error::Eval(format!(
            "openai codec does not support content part type {other}"
        ))),
    }
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
            .map_err(|err| Error::Eval(format!("openai codec invalid {key}: {err}"))),
        other => {
            let json_number = match other {
                Expr::String(text) => serde_json::from_str::<Value>(text).ok(),
                _ => None,
            };
            json_number
                .as_ref()
                .and_then(json_number_to_u64)
                .ok_or_else(|| Error::Eval(format!("openai codec field {key} must be a number")))
                .map(Some)
        }
    }
}

fn marker_is_true(expr: &Expr, name: &str) -> bool {
    let Expr::Map(entries) = expr else {
        return false;
    };
    entries.iter().any(|(key, value)| {
        matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == name)
            && matches!(value, Expr::Bool(true))
    })
}

fn symbol_field(entries: &[(Expr, Expr)], key: &str) -> Result<String> {
    match map_field(entries, key)? {
        Expr::Symbol(symbol) => Ok(symbol.name.as_ref().to_owned()),
        _ => Err(Error::Eval(format!(
            "openai codec field {key} must be a symbol"
        ))),
    }
}

fn string_field(entries: &[(Expr, Expr)], key: &str) -> Result<String> {
    match map_field(entries, key)? {
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::Eval(format!(
            "openai codec field {key} must be a string"
        ))),
    }
}

fn list_field(expr: &Expr) -> Result<&[Expr]> {
    match expr {
        Expr::List(items) => Ok(items),
        _ => Err(Error::Eval("openai codec field must be a list".to_owned())),
    }
}

fn flatten_expr(expr: &Expr) -> String {
    match expr {
        Expr::Nil => "nil".to_owned(),
        Expr::Bool(flag) => flag.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        Expr::Bytes(bytes) => format!("{bytes:?}"),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().map(flatten_expr).collect::<Vec<_>>().join(" ")
        }
        Expr::Map(entries) => entries
            .iter()
            .map(|(key, value)| format!("{} {}", flatten_expr(key), flatten_expr(value)))
            .collect::<Vec<_>>()
            .join(" "),
        Expr::Call { operator, args } => std::iter::once(flatten_expr(operator))
            .chain(args.iter().map(flatten_expr))
            .collect::<Vec<_>>()
            .join(" "),
        Expr::Infix {
            operator,
            left,
            right,
        } => format!(
            "{} {} {}",
            flatten_expr(left),
            operator,
            flatten_expr(right)
        ),
        Expr::Prefix { operator, arg } => format!("{operator} {}", flatten_expr(arg)),
        Expr::Postfix { operator, arg } => format!("{} {operator}", flatten_expr(arg)),
        Expr::Quote { expr, .. } | Expr::Annotated { expr, .. } => flatten_expr(expr),
        Expr::Extension { tag, payload } => format!("{tag} {}", flatten_expr(payload)),
    }
}
