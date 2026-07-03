use super::super::model::{AgentComponent, RecorderBackend};
use crate::{
    memory::{
        append_memory_log, flatten_expr_text, lock_entries, memory_entries_append,
        memory_entries_recent, memory_entries_search, memory_entries_snapshot,
    },
    util::{symbol_of, u32_from_expr},
};
use sim_kernel::{Cx, Error, Expr, ReadPolicy, Result, Symbol, Value};
use sim_lib_server::{FrameKind, ServerFrame, eval_request_from_frame};

pub(in crate::components) fn answer_recorder(
    cx: &mut Cx,
    _component: &AgentComponent,
    backend: &RecorderBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    match frame.kind {
        FrameKind::Notify => {
            let trace = direct_trace_entry(cx, &frame)?.unwrap_or(frame_trace_expr(cx, &frame)?);
            recorder_append(backend, trace)?;
            Ok(frame)
        }
        FrameKind::Request => {
            let consistency = frame.envelope.consistency;
            let request = eval_request_from_frame(cx, &frame)?;
            let value = recorder_request_value(cx, backend, request.expr)?;
            crate::reply::reply_frame(cx, &frame, value, consistency)
        }
        _ => Err(Error::Eval(
            "recorder expects request or notify frames".to_owned(),
        )),
    }
}

fn direct_trace_entry(cx: &mut Cx, frame: &ServerFrame) -> Result<Option<Expr>> {
    let payload = frame.decode_expr(cx, ReadPolicy::default())?;
    Ok(match &payload {
        Expr::Map(entries)
            if entries.iter().any(|(key, value)| {
                matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "trace-entry")
                    && matches!(value, Expr::Bool(true))
            }) =>
        {
            Some(crate::model_privacy::redact_trace_entry(payload)?)
        }
        _ => None,
    })
}

fn recorder_append(backend: &RecorderBackend, expr: Expr) -> Result<()> {
    match backend {
        RecorderBackend::Journal { path, entries } | RecorderBackend::Audit { path, entries } => {
            memory_entries_append(entries, expr.clone())?;
            crate::agents::remember_recorded_trace(&expr)?;
            if let Some(path) = path {
                append_memory_log(path, &expr)?;
            }
            Ok(())
        }
        RecorderBackend::Prometheus { entries, .. } => {
            memory_entries_append(entries, expr.clone())?;
            crate::agents::remember_recorded_trace(&expr)
        }
    }
}

fn frame_trace_expr(cx: &mut Cx, frame: &ServerFrame) -> Result<Expr> {
    let payload = frame.decode_expr(cx, ReadPolicy::default())?;
    crate::model_privacy::redact_trace_entry(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("kind")),
            Expr::Symbol(frame.kind.as_symbol()),
        ),
        (
            Expr::Symbol(Symbol::new("codec")),
            Expr::Symbol(frame.codec.clone()),
        ),
        (
            Expr::Symbol(Symbol::new("role")),
            frame
                .envelope
                .role
                .clone()
                .map(Expr::Symbol)
                .unwrap_or(Expr::Nil),
        ),
        (Expr::Symbol(Symbol::new("payload")), payload),
    ]))
}

fn recorder_request_value(cx: &mut Cx, backend: &RecorderBackend, expr: Expr) -> Result<Value> {
    match backend {
        RecorderBackend::Prometheus { namespace, entries } => {
            let _ = expr;
            cx.factory().string(render_prometheus_scrape(
                namespace,
                &lock_entries(entries, "recorder metrics")?,
            )?)
        }
        RecorderBackend::Journal { entries, .. } | RecorderBackend::Audit { entries, .. } => {
            match expr {
                Expr::List(items) | Expr::Vector(items) => {
                    let Some((head, tail)) = items.split_first() else {
                        return cx.factory().list(Vec::new());
                    };
                    let op = symbol_of(head, "recorder request expects a symbol op")?;
                    match op.to_string().as_str() {
                        "snapshot" | "journal" => {
                            cx.factory().expr(memory_entries_snapshot(entries)?)
                        }
                        "recent" => {
                            let count = tail
                                .first()
                                .map(|expr| u32_from_expr(expr, "recorder recent expects a count"))
                                .transpose()?
                                .unwrap_or(10);
                            let recent = memory_entries_recent(cx, entries, count)?;
                            cx.factory().list(recent)
                        }
                        "audit" | "search" => {
                            let query = tail.first().cloned().unwrap_or(Expr::Nil);
                            let count = tail
                                .get(1)
                                .map(|expr| u32_from_expr(expr, "recorder audit expects a count"))
                                .transpose()?
                                .unwrap_or(10);
                            let found = memory_entries_search(cx, entries, query, count)?;
                            cx.factory().list(found)
                        }
                        _ => cx.factory().expr(memory_entries_snapshot(entries)?),
                    }
                }
                _ => cx.factory().expr(memory_entries_snapshot(entries)?),
            }
        }
    }
}

fn render_prometheus_scrape(namespace: &str, entries: &[Expr]) -> Result<String> {
    let mut out = String::new();
    let total = u64::try_from(entries.len()).unwrap_or(u64::MAX);
    out.push_str(&format!(
        "# HELP {namespace}_frames_total Frames recorded by the agent fabric.\n"
    ));
    out.push_str(&format!("# TYPE {namespace}_frames_total counter\n"));
    out.push_str(&format!("{namespace}_frames_total {total}\n"));

    let mut roles = std::collections::BTreeMap::<String, u64>::new();
    let mut tools = std::collections::BTreeMap::<String, u64>::new();
    for entry in entries {
        if let Some(role) = trace_field(entry, "role").and_then(expr_string_label) {
            *roles.entry(role).or_default() += 1;
        }
        if let Some(tool) = extract_tool_label(entry) {
            *tools.entry(tool).or_default() += 1;
        }
    }
    for (role, count) in roles {
        out.push_str(&format!(
            "{namespace}_frames_total{{role=\"{}\"}} {count}\n",
            escape_prometheus_label(&role)
        ));
    }
    out.push_str(&format!(
        "# HELP {namespace}_tool_calls_total Tool calls recorded.\n"
    ));
    out.push_str(&format!("# TYPE {namespace}_tool_calls_total counter\n"));
    for (tool, count) in tools {
        out.push_str(&format!(
            "{namespace}_tool_calls_total{{tool=\"{}\"}} {count}\n",
            escape_prometheus_label(&tool)
        ));
    }
    Ok(out)
}

fn trace_field<'a>(entry: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = entry else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn expr_string_label(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Nil => None,
        Expr::String(text) => Some(text.clone()),
        Expr::Symbol(symbol) => Some(symbol.to_string()),
        _ => Some(flatten_expr_text(expr)),
    }
}

fn extract_tool_label(entry: &Expr) -> Option<String> {
    let payload = trace_field(entry, "payload")?;
    if let Some(tool) = payload_map_field(payload, "tool").and_then(expr_string_label) {
        return Some(tool);
    }
    match payload {
        Expr::List(items) | Expr::Vector(items) => items.first().and_then(expr_string_label),
        Expr::Symbol(symbol) => Some(symbol.to_string()),
        _ => None,
    }
}

fn payload_map_field<'a>(expr: &'a Expr, key: &str) -> Option<&'a Expr> {
    let Expr::Map(entries) = expr else {
        return None;
    };
    entries.iter().find_map(|(field, value)| match field {
        Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(value),
        _ => None,
    })
}

fn escape_prometheus_label(label: &str) -> String {
    let mut escaped = String::with_capacity(label.len());
    for ch in label.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
