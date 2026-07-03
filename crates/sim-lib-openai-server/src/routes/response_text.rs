use sim_kernel::Expr;

use super::errors::OpenAiRouteError;

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

pub(crate) fn response_delta_chunks(response: &Expr, streaming: bool) -> RouteResult<Vec<String>> {
    let output_text = response_output_text(response)?;
    if streaming {
        Ok(stream_text_chunks(&output_text))
    } else {
        Ok(vec![output_text])
    }
}

fn response_output_text(response: &Expr) -> RouteResult<String> {
    let content = response_field(response, "content")
        .ok_or_else(|| OpenAiRouteError::internal_message("model response missing content"))?;
    let Expr::List(parts) = content else {
        return Err(OpenAiRouteError::internal_message(
            "model response content must be a list",
        ));
    };
    parts
        .iter()
        .map(response_text_part)
        .collect::<RouteResult<Vec<_>>>()
        .map(|parts| parts.join(""))
}

fn response_text_part(expr: &Expr) -> RouteResult<String> {
    let Expr::Map(entries) = expr else {
        return Err(OpenAiRouteError::internal_message(
            "model response content part must be a map",
        ));
    };
    let part_type = entries.iter().find_map(|(key, value)| match (key, value) {
        (Expr::Symbol(symbol), Expr::Symbol(value)) if symbol.name.as_ref() == "type" => {
            Some(value.name.as_ref())
        }
        _ => None,
    });
    if part_type != Some("text") {
        return Err(OpenAiRouteError::internal_message(
            "streaming fixture responses support only text content",
        ));
    }
    entries
        .iter()
        .find_map(|(key, value)| match (key, value) {
            (Expr::Symbol(symbol), Expr::String(text)) if symbol.name.as_ref() == "text" => {
                Some(text.clone())
            }
            _ => None,
        })
        .ok_or_else(|| OpenAiRouteError::internal_message("text content part missing text"))
}

fn response_field<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(key, value)| match key {
        Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name => {
            Some(value)
        }
        _ => None,
    })
}

fn stream_text_chunks(text: &str) -> Vec<String> {
    text.split_whitespace()
        .enumerate()
        .map(|(index, part)| {
            if index == 0 {
                part.to_owned()
            } else {
                format!(" {part}")
            }
        })
        .collect()
}
