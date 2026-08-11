use serde_json::{Value, json};
pub use sim_codec_chat::{encode_openai_request, encode_openai_response};
use sim_codec_json::json_number_to_u64;
use sim_kernel::{Error, Expr, Result};
use sim_value::access::{entry_field, entry_required_str_any, entry_required_sym_any};

use crate::objects::canonical_json_bytes;

/// Encodes a response transcript into OpenAI Responses API response JSON,
/// stamping it with `response_id` and `created_at_ms`.
pub fn encode_openai_responses_response(
    expr: &Expr,
    response_id: &str,
    created_at_ms: u64,
) -> Result<Vec<u8>> {
    let value = responses_response_json(expr, response_id, created_at_ms)?;
    Ok(canonical_json_bytes(value))
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
    let total = optional_u64_field(fields, "total-tokens")?.or_else(|| {
        prompt
            .zip(completion)
            .map(|(left, right)| left.saturating_add(right))
    });
    Ok(json!({
        "prompt_tokens": prompt.unwrap_or(0),
        "completion_tokens": completion.unwrap_or(0),
        "total_tokens": total.unwrap_or(0),
    }))
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

fn symbol_field(entries: &[(Expr, Expr)], key: &str) -> Result<String> {
    entry_required_sym_any(entries, key, "openai codec symbol field")
        .map(|symbol| symbol.name.as_ref().to_owned())
}

fn string_field(entries: &[(Expr, Expr)], key: &str) -> Result<String> {
    entry_required_str_any(entries, key, "openai codec string field").map(str::to_owned)
}

fn list_field(expr: &Expr) -> Result<&[Expr]> {
    match expr {
        Expr::List(items) => Ok(items),
        _ => Err(Error::Eval("openai codec field must be a list".to_owned())),
    }
}

fn map_field<'a>(entries: &'a [(Expr, Expr)], key: &str) -> Result<&'a Expr> {
    entry_field(entries, key)
        .ok_or_else(|| Error::Eval(format!("openai codec missing {key} field")))
}
