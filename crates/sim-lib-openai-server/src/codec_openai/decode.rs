use serde_json::{Map, Value};
use sim_codec::Input;
use sim_codec_chat::{model_response_expr, text_part, usage_record, validate_chat_transcript};
use sim_codec_json::{JsonProjectionMode, json_number_to_u64, project_json_to_expr};
use sim_kernel::{CodecId, Error, Expr, Result, Symbol};

use crate::codec_openai::{codec_error, input_text};

/// Decodes OpenAI request JSON into a validated SIM chat-transcript expression.
pub fn decode_openai_request(input: Input) -> Result<Expr> {
    decode_openai_request_for_codec(CodecId(0), input)
}

pub(crate) fn decode_openai_request_for_codec(codec: CodecId, input: Input) -> Result<Expr> {
    let source = input_text(codec, input)?;
    let value = serde_json::from_str::<Value>(&source).map_err(|err| codec_error(codec, err))?;
    let request = value
        .as_object()
        .ok_or_else(|| codec_error(codec, "openai request must be a json object"))?;
    let model = string_member(codec, request, "model")?.to_owned();
    let (task, messages) = request_task_and_messages(codec, request)?;
    let mut entries = vec![
        (Expr::Symbol(Symbol::new("model-request")), Expr::Bool(true)),
        (Expr::Symbol(Symbol::new("task")), task),
        (Expr::Symbol(Symbol::new("messages")), Expr::List(messages)),
        (Expr::Symbol(Symbol::new("model")), Expr::String(model)),
    ];
    if let Some(stream) = request.get("stream").and_then(Value::as_bool) {
        entries.push((Expr::Symbol(Symbol::new("stream")), Expr::Bool(stream)));
    }
    if let Some(privacy) = request.get("privacy").and_then(Value::as_str) {
        entries.push((
            Expr::Symbol(Symbol::new("privacy")),
            Expr::String(privacy.to_owned()),
        ));
    }
    if let Some(budget) = request.get("budget") {
        entries.push((
            Expr::Symbol(Symbol::new("budget")),
            project_json_to_expr(budget, JsonProjectionMode::UntaggedInterop),
        ));
    }
    if let Some(max_tokens) = request
        .get("max_tokens")
        .or_else(|| request.get("max-tokens"))
    {
        entries.push((
            Expr::Symbol(Symbol::new("max-tokens")),
            project_json_to_expr(max_tokens, JsonProjectionMode::UntaggedInterop),
        ));
    }
    if let Some(tools) = request.get("tools") {
        entries.push((
            Expr::Symbol(Symbol::new("tools")),
            project_json_to_expr(tools, JsonProjectionMode::UntaggedInterop),
        ));
    }
    if let Some(choice) = request
        .get("tool_choice")
        .or_else(|| request.get("tool-choice"))
    {
        entries.push((
            Expr::Symbol(Symbol::new("tool-choice")),
            project_json_to_expr(choice, JsonProjectionMode::UntaggedInterop),
        ));
    }
    let expr = Expr::Map(entries);
    validate_chat_transcript(&expr)?;
    Ok(expr)
}

fn request_task_and_messages(
    codec: CodecId,
    request: &Map<String, Value>,
) -> Result<(Expr, Vec<Expr>)> {
    if let Some(messages) = request.get("messages").and_then(Value::as_array) {
        let (task_message, prior_messages) = messages
            .split_last()
            .ok_or_else(|| codec_error(codec, "openai request messages must not be empty"))?;
        return Ok((
            Expr::String(message_text(codec, task_message)?),
            prior_messages
                .iter()
                .map(|message| message_expr(codec, message))
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    let input = request
        .get("input")
        .ok_or_else(|| codec_error(codec, "openai request missing input"))?;
    Ok((Expr::String(input_text_value(codec, input)?), Vec::new()))
}

fn input_text_value(codec: CodecId, value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        _ => Err(codec_error(codec, "openai request input must be a string")),
    }
}

/// Decodes an OpenAI chat-completion response body into a SIM model-response
/// transcript, attributing it to `runner`/`model` and optionally embedding the
/// raw provider JSON when `include_raw` is set.
pub fn decode_openai_response(
    runner: Symbol,
    model: &str,
    body: &[u8],
    include_raw: bool,
) -> Result<Expr> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|err| Error::Eval(format!("openai codec returned invalid json: {err}")))?;
    let response = value
        .as_object()
        .ok_or_else(|| Error::Eval("openai response must be a json object".to_owned()))?;
    let choice = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Eval("openai response missing choices[0]".to_owned()))?;
    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Eval("openai response missing choices[0].message".to_owned()))?;
    let stop_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("stop");
    let mut entries = match model_response_expr(
        runner,
        model,
        message_content(message)?,
        Symbol::new(stop_reason),
    ) {
        Expr::Map(entries) => entries,
        _ => unreachable!("model_response_expr always returns a map"),
    };
    if let Some(usage) = usage_expr(response)? {
        entries.push((Expr::Symbol(Symbol::new("usage")), usage));
    }
    if include_raw {
        entries.push((
            Expr::Symbol(Symbol::new("raw-provider-response")),
            project_json_to_expr(&value, JsonProjectionMode::UntaggedInterop),
        ));
    }
    Ok(Expr::Map(entries))
}

fn message_expr(codec: CodecId, value: &Value) -> Result<Expr> {
    let object = value
        .as_object()
        .ok_or_else(|| codec_error(codec, "openai message must be an object"))?;
    let role = string_member(codec, object, "role")?;
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("role")),
            Expr::Symbol(Symbol::new(role)),
        ),
        (
            Expr::Symbol(Symbol::new("content")),
            Expr::List(content_parts(codec, object.get("content"))?),
        ),
    ]))
}

fn message_text(codec: CodecId, value: &Value) -> Result<String> {
    let object = value
        .as_object()
        .ok_or_else(|| codec_error(codec, "openai task message must be an object"))?;
    let role = string_member(codec, object, "role")?;
    if role != "user" {
        return Err(codec_error(
            codec,
            "openai request final message must have role user",
        ));
    }
    let parts = content_parts(codec, object.get("content"))?;
    parts
        .iter()
        .map(text_from_part)
        .collect::<Result<Vec<_>>>()
        .map(|items| items.join("\n"))
}

fn content_parts(codec: CodecId, content: Option<&Value>) -> Result<Vec<Expr>> {
    match content {
        Some(Value::String(text)) => Ok(vec![text_part(text)]),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| request_part_from_json(codec, part))
            .collect(),
        Some(Value::Null) | None => Ok(Vec::new()),
        _ => Err(codec_error(
            codec,
            "openai message content must be string, array, or null",
        )),
    }
}

fn request_part_from_json(codec: CodecId, value: &Value) -> Result<Expr> {
    let object = value
        .as_object()
        .ok_or_else(|| codec_error(codec, "openai content part must be an object"))?;
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("text");
    match kind {
        "text" => Ok(text_part(
            object
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| codec_error(codec, "openai text content part missing text"))?,
        )),
        other => Err(codec_error(
            codec,
            format!("openai content part type {other} is not supported"),
        )),
    }
}

fn message_content(message: &Map<String, Value>) -> Result<Vec<Expr>> {
    match message.get("content") {
        Some(Value::String(text)) => Ok(vec![text_part(text)]),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(response_part_from_json)
            .collect::<Result<Vec<_>>>(),
        Some(Value::Null) | None => Ok(Vec::new()),
        _ => Err(Error::Eval(
            "openai response message content must be string, array, or null".to_owned(),
        )),
    }
}

fn response_part_from_json(value: &Value) -> Result<Expr> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Eval("openai response content part must be an object".to_owned()))?;
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("text");
    match kind {
        "text" => Ok(text_part(
            object
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Eval("openai response text part missing text".to_owned()))?,
        )),
        other => Err(Error::Eval(format!(
            "openai response content part type {other} is not supported"
        ))),
    }
}

fn usage_expr(response: &Map<String, Value>) -> Result<Option<Expr>> {
    let Some(usage) = response.get("usage").and_then(Value::as_object) else {
        return Ok(None);
    };
    let input = usage.get("prompt_tokens").and_then(json_number_to_u64);
    let output = usage.get("completion_tokens").and_then(json_number_to_u64);
    let total = usage.get("total_tokens").and_then(json_number_to_u64);
    // OpenAI emits a usage object: keep an (possibly empty) usage map present.
    Ok(Some(Expr::Map(usage_record(input, output, total))))
}

fn text_from_part(part: &Expr) -> Result<String> {
    let Expr::Map(entries) = part else {
        return Err(Error::Eval(
            "openai content part transcript must be a map".to_owned(),
        ));
    };
    match map_field(entries, "text")? {
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::Eval(
            "openai text content part text field must be a string".to_owned(),
        )),
    }
}

fn string_member<'a>(
    codec: CodecId,
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| codec_error(codec, format!("openai request missing string {name}")))
}

pub(crate) fn map_field<'a>(entries: &'a [(Expr, Expr)], key: &str) -> Result<&'a Expr> {
    // OpenAI records use unqualified keys, so the guarded entry_field is
    // equivalent to the old agnostic lookup (verified by the codec snapshot tests).
    sim_value::access::entry_field(entries, key)
        .ok_or_else(|| Error::Eval(format!("openai codec missing {key} field")))
}
