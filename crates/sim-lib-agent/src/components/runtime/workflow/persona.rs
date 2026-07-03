use crate::components::model::{AgentComponent, PersonaBackend};
use crate::components::runtime::stream_transform::transform_data_payloads;
use crate::memory::flatten_expr_text;
use crate::util::expr_to_value;
use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol};
use sim_lib_server::{FrameKind, ServerFrame, eval_request_from_frame};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

pub(in crate::components) fn answer_persona(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &PersonaBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    match frame.kind {
        FrameKind::Request => answer_persona_request(cx, component, backend, frame),
        _ => answer_persona_stream(cx, component, backend, frame),
    }
}

fn answer_persona_request(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &PersonaBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    let consistency = frame.envelope.consistency;
    let request = eval_request_from_frame(cx, &frame)?;
    let shaped = persona_shape_expr(cx, backend, request.expr)?;
    let value = expr_to_value(cx, &shaped)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
        .map_err(|err| Error::Eval(format!("{} failed to shape: {err}", component.symbol)))
}

fn answer_persona_stream(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &PersonaBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    transform_data_payloads(cx, frame, |cx, expr| persona_shape_expr(cx, backend, expr)).map_err(
        |err| {
            Error::Eval(format!(
                "{} only answers request or stream transform frames: {err}",
                component.symbol
            ))
        },
    )
}

fn persona_shape_expr(cx: &mut Cx, backend: &PersonaBackend, expr: Expr) -> Result<Expr> {
    match backend {
        PersonaBackend::Style {
            voice,
            prefix,
            suffix,
            max_words,
        } => Ok(Expr::String(shape_output(
            &shape_input(&flatten_expr_text(&expr), prefix, suffix),
            voice,
            prefix,
            suffix,
            *max_words,
        ))),
        PersonaBackend::Language { language, glossary } => Ok(Expr::String(language_transform(
            &flatten_expr_text(&expr),
            language,
            glossary,
        ))),
        PersonaBackend::Translator {
            from,
            to,
            table,
            command,
        } => translator_transform(cx, &flatten_expr_text(&expr), from, to, table, command),
    }
}

fn shape_input(text: &str, prefix: &str, suffix: &str) -> String {
    let mut shaped = normalize_whitespace(text);
    if !prefix.is_empty() && shaped.starts_with(prefix) {
        shaped = shaped[prefix.len()..].trim().to_owned();
    }
    if !suffix.is_empty() && shaped.ends_with(suffix) {
        shaped = shaped[..shaped.len().saturating_sub(suffix.len())]
            .trim()
            .to_owned();
    }
    shaped
}

fn shape_output(text: &str, voice: &str, prefix: &str, suffix: &str, max_words: usize) -> String {
    let mut words = tokenize_words(text);
    if words.len() > max_words {
        words.truncate(max_words);
    }
    let mut shaped = words.join(" ");
    if voice == "terse" && !shaped.ends_with('.') {
        shaped.push('.');
    }
    let mut parts = Vec::new();
    if !prefix.is_empty() {
        parts.push(prefix.trim().to_owned());
    }
    if !shaped.is_empty() {
        parts.push(shaped);
    }
    let mut output = parts.join(" ");
    if !suffix.is_empty() {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(suffix.trim());
    }
    normalize_whitespace(&output)
}

fn language_transform(text: &str, language: &str, glossary: &[(String, String)]) -> String {
    let glossary = glossary_lookup(glossary);
    text.split_whitespace()
        .map(|token| {
            let key = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase();
            glossary
                .get(&key)
                .cloned()
                .unwrap_or_else(|| format!("[{language}:{token}]"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn translator_transform(
    cx: &mut Cx,
    text: &str,
    from: &str,
    to: &str,
    table: &[(String, String)],
    command: &Option<String>,
) -> Result<Expr> {
    if let Some(command) = command {
        cx.require(&CapabilityName::new("sandbox-subprocess"))?;
        return run_translation_command(text, from, to, command);
    }

    let lookup = glossary_lookup(table);
    let tokens = text.split_whitespace().collect::<Vec<_>>();
    let mut translated = 0usize;
    let rendered = tokens
        .iter()
        .map(|token| {
            let key = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase();
            if let Some(value) = lookup.get(&key) {
                translated += 1;
                value.clone()
            } else {
                (*token).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let coverage = if tokens.is_empty() {
        0.0
    } else {
        translated as f32 / tokens.len() as f32
    };
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("text")),
            Expr::String(normalize_whitespace(&rendered)),
        ),
        (
            Expr::Symbol(Symbol::new("coverage")),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: coverage.to_string(),
            }),
        ),
        (
            Expr::Symbol(Symbol::new("from")),
            Expr::String(from.to_owned()),
        ),
        (Expr::Symbol(Symbol::new("to")), Expr::String(to.to_owned())),
    ]))
}

fn run_translation_command(text: &str, from: &str, to: &str, command: &str) -> Result<Expr> {
    let mut child = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| Error::Eval(format!("translator command failed to start: {err}")))?;
    let payload = format!("{from}\n{to}\n{text}\n");
    child
        .stdin
        .as_mut()
        .ok_or_else(|| Error::Eval("translator command stdin unavailable".to_owned()))?
        .write_all(payload.as_bytes())
        .map_err(|err| Error::Eval(format!("translator command stdin failed: {err}")))?;
    let output = child
        .wait_with_output()
        .map_err(|err| Error::Eval(format!("translator command failed: {err}")))?;
    if !output.status.success() {
        return Err(Error::Eval(format!(
            "translator command exited with status {}",
            output.status
        )));
    }
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("text")),
            Expr::String(normalize_whitespace(&String::from_utf8_lossy(
                &output.stdout,
            ))),
        ),
        (
            Expr::Symbol(Symbol::new("coverage")),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "f64"),
                canonical: "1".to_owned(),
            }),
        ),
    ]))
}

fn glossary_lookup(entries: &[(String, String)]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| (key.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokenize_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| word.to_owned())
        .collect()
}
