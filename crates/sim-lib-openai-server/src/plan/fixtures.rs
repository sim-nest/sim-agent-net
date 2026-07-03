use sim_codec_chat::text_part;
use sim_kernel::{Expr, Symbol};
use sim_lib_agent_runner_core::{ModelRequest, ModelResponse, ModelUsage};

pub(crate) fn fixture_echo_response(model: &str, request: &ModelRequest) -> ModelResponse {
    fixture_static_response(model, &request_text(request))
}

pub(crate) fn fixture_tool_call_response(
    model: &str,
    request: &ModelRequest,
    repeat: bool,
) -> ModelResponse {
    let tool_results = tool_result_texts(request);
    if !repeat && !tool_results.is_empty() {
        return fixture_static_response(model, &tool_results.join("\n"));
    }
    ModelResponse::new(
        Symbol::new(model.to_owned()),
        model,
        vec![tool_call_part(
            selected_tool_name(request),
            fixture_arguments_json(request),
        )],
        Symbol::new("tool-call"),
    )
}

pub(crate) fn fixture_static_response(model: &str, text: &str) -> ModelResponse {
    let mut response = ModelResponse::new(
        Symbol::new(model.to_owned()),
        model,
        vec![text_part(text)],
        Symbol::new("stop"),
    );
    let tokens = text.split_whitespace().count() as u64;
    response.usage = Some(ModelUsage {
        input_tokens: Some(tokens),
        output_tokens: Some(tokens),
        latency_ms: Some(if model == "fixture/slow-echo" {
            1_000
        } else {
            0
        }),
        cost_usd: Some(0.0),
        extra: Vec::new(),
    });
    response
}

pub(crate) fn request_text(request: &ModelRequest) -> String {
    let mut parts = Vec::new();
    collect_text(&request.task, &mut parts);
    for message in &request.messages {
        collect_text(message, &mut parts);
    }
    if parts.is_empty() {
        "fixture echo".to_owned()
    } else {
        parts.join(" ")
    }
}

pub(crate) fn response_text(response: &ModelResponse) -> String {
    let mut parts = Vec::new();
    for item in &response.content {
        collect_text(item, &mut parts);
    }
    parts.join(" ")
}

pub(crate) fn response_summary(response: &ModelResponse) -> Expr {
    Expr::Map(vec![
        field("runner", Expr::Symbol(response.runner.clone())),
        field("model", Expr::String(response.model.clone())),
        field("text", Expr::String(response_text(response))),
    ])
}

fn collect_text(expr: &Expr, parts: &mut Vec<String>) {
    match expr {
        Expr::String(text) => parts.push(text.clone()),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            for item in items {
                collect_text(item, parts);
            }
        }
        Expr::Map(entries) => {
            for (_, value) in entries {
                collect_text(value, parts);
            }
        }
        Expr::Call { operator, args } => {
            collect_text(operator, parts);
            for arg in args {
                collect_text(arg, parts);
            }
        }
        Expr::Infix { left, right, .. } => {
            collect_text(left, parts);
            collect_text(right, parts);
        }
        Expr::Prefix { arg, .. } | Expr::Postfix { arg, .. } | Expr::Quote { expr: arg, .. } => {
            collect_text(arg, parts);
        }
        Expr::Annotated { expr, annotations } => {
            collect_text(expr, parts);
            for (_, value) in annotations {
                collect_text(value, parts);
            }
        }
        Expr::Extension { payload, .. } => collect_text(payload, parts),
        Expr::Nil
        | Expr::Bool(_)
        | Expr::Number(_)
        | Expr::Symbol(_)
        | Expr::Local(_)
        | Expr::Bytes(_) => {}
    }
}

use sim_value::build::entry as field;

fn tool_call_part(name: String, arguments_json: String) -> Expr {
    Expr::Map(vec![
        field("type", Expr::Symbol(Symbol::new("tool-call"))),
        field("id", Expr::String(format!("call_{name}_000001"))),
        field("name", Expr::String(name)),
        field("arguments", Expr::String(arguments_json)),
    ])
}

fn selected_tool_name(request: &ModelRequest) -> String {
    request
        .extra
        .iter()
        .find_map(|(key, value)| {
            matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "tool-choice")
                .then_some(value)
        })
        .and_then(tool_choice_name)
        .or_else(|| {
            request
                .extra
                .iter()
                .find_map(|(key, value)| {
                    matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "tools")
                        .then_some(value)
                })
                .and_then(first_tool_name)
        })
        .unwrap_or_else(|| "tool_echo".to_owned())
}

fn tool_choice_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::String(name) if name != "auto" && name != "none" && name != "required" => {
            Some(name.clone())
        }
        Expr::Map(entries) => entries.iter().find_map(|(key, value)| {
            if !matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "function") {
                return None;
            }
            let Expr::Map(function) = value else {
                return None;
            };
            string_field(function, "name")
        }),
        _ => None,
    }
}

fn first_tool_name(expr: &Expr) -> Option<String> {
    let Expr::List(tools) = expr else {
        return None;
    };
    tools.iter().find_map(|tool| {
        let Expr::Map(entries) = tool else {
            return None;
        };
        entries.iter().find_map(|(key, value)| {
            if !matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "function") {
                return None;
            }
            let Expr::Map(function) = value else {
                return None;
            };
            string_field(function, "name")
        })
    })
}

fn fixture_arguments_json(request: &ModelRequest) -> String {
    let text = match &request.task {
        Expr::String(text) => text.clone(),
        _ => request_text(request),
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(_)) => text,
        _ => serde_json::json!({ "text": text }).to_string(),
    }
}

fn tool_result_texts(request: &ModelRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| message_role(message).as_deref() == Some("tool"))
        .map(|message| {
            let mut parts = Vec::new();
            collect_text(message, &mut parts);
            parts.join(" ")
        })
        .collect()
}

fn message_role(expr: &Expr) -> Option<String> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match (key, value) {
        (Expr::Symbol(key), Expr::Symbol(value))
            if key.namespace.is_none() && key.name.as_ref() == "role" =>
        {
            Some(value.name.as_ref().to_owned())
        }
        _ => None,
    })
}

fn string_field(entries: &[(Expr, Expr)], name: &str) -> Option<String> {
    sim_value::access::entry_field(entries, name)
        .and_then(sim_value::access::as_str)
        .map(str::to_owned)
}
