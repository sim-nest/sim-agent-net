use crate::{
    ProcessRunner,
    process::{run_command, stream_command_lines},
};
use sim_codec_chat::text_part;
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{
    ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelUsage,
};

pub(crate) fn infer(runner: &ProcessRunner, request: ModelRequest) -> Result<ModelResponse> {
    let prompt = prompt_text(&request);
    let stdout = run_command(
        &runner.command,
        prompt.into_bytes(),
        "runner/process",
        runner.timeout,
        runner.max_output_bytes,
    )?;
    let text = String::from_utf8(stdout)
        .map_err(|_| sim_kernel::Error::Eval("runner/process returned non-utf8 text".to_owned()))?;
    Ok(ModelResponse::new(
        runner.runner.clone(),
        runner.model.clone(),
        vec![Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("type")),
                Expr::Symbol(Symbol::new("text")),
            ),
            (
                Expr::Symbol(Symbol::new("text")),
                Expr::String(text.clone()),
            ),
        ])],
        Symbol::new("stop"),
    )
    .with_text(text))
}

pub(crate) fn infer_stream(
    runner: &ProcessRunner,
    request: ModelRequest,
    sink: &mut dyn ModelEventSink,
) -> Result<ModelResponse> {
    let prompt = prompt_text(&request);
    let span_id = Expr::String("line-text".to_owned());
    sink.emit(ModelEvent::start(
        runner.runner.clone(),
        runner.model.clone(),
        span_id.clone(),
    ))?;
    let mut line_count = 0_u64;
    let stdout = stream_command_lines(
        &runner.command,
        prompt.into_bytes(),
        "runner/process",
        runner.timeout,
        runner.max_output_bytes,
        |line| {
            let line = std::str::from_utf8(line)
                .map_err(|_| Error::Eval("runner/process returned non-utf8 text".to_owned()))?;
            let line = line.trim_end_matches(['\r', '\n']);
            line_count += 1;
            sink.emit(ModelEvent::delta_text(
                runner.runner.clone(),
                runner.model.clone(),
                span_id.clone(),
                line,
            ))
        },
    )?;
    let text = String::from_utf8(stdout)
        .map_err(|_| Error::Eval("runner/process returned non-utf8 text".to_owned()))?;
    let response = ModelResponse::new(
        runner.runner.clone(),
        runner.model.clone(),
        vec![text_part(&text)],
        Symbol::new("stop"),
    )
    .with_text(text.clone());
    sink.emit(ModelEvent::usage(
        runner.runner.clone(),
        runner.model.clone(),
        span_id,
        ModelUsage {
            output_tokens: Some(line_count),
            ..ModelUsage::default()
        },
    ))?;
    sink.emit(ModelEvent::final_of(&response))?;
    Ok(response)
}

trait WithText {
    fn with_text(self, text: String) -> Self;
}

impl WithText for ModelResponse {
    fn with_text(mut self, text: String) -> Self {
        self.extra
            .push((Expr::Symbol(Symbol::new("text")), Expr::String(text)));
        self
    }
}

fn prompt_text(request: &ModelRequest) -> String {
    let mut parts = vec![flatten_expr(&request.task)];
    parts.extend(request.messages.iter().map(flatten_expr));
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn flatten_expr(expr: &Expr) -> String {
    match expr {
        Expr::Nil => String::new(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.to_string(),
        Expr::String(text) => text.clone(),
        Expr::Bytes(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().map(flatten_expr).collect::<Vec<_>>().join(" ")
        }
        Expr::Map(entries) => entries
            .iter()
            .map(|(key, value)| {
                let key = flatten_expr(key);
                let value = flatten_expr(value);
                if key.is_empty() {
                    value
                } else if value.is_empty() {
                    key
                } else {
                    format!("{key}: {value}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Expr::Call { operator, args } => {
            let mut parts = vec![flatten_expr(operator)];
            parts.extend(args.iter().map(flatten_expr));
            parts.join(" ")
        }
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
        Expr::Quote { expr, .. } => flatten_expr(expr),
        Expr::Annotated { expr, .. } => flatten_expr(expr),
        Expr::Extension { tag, payload } => format!("{tag} {}", flatten_expr(payload)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProcessProtocol, ProcessRunner};
    use sim_lib_agent_runner_core::VecEventSink;
    use std::time::Duration;

    #[test]
    fn line_text_streams_deltas_before_final_response() {
        let runner = ProcessRunner::new(
            Symbol::new("line-runner"),
            "line/model",
            "printf 'one\\ntwo\\n'",
            ProcessProtocol::LineText,
            Duration::from_secs(1),
            1024,
        );
        let request = ModelRequest::new(Expr::String("prompt".to_owned()), Vec::new());
        let mut sink = VecEventSink::new();

        let response = infer_stream(&runner, request, &mut sink).unwrap();
        let events = sink.into_events();
        let kinds = events
            .iter()
            .map(|event| event.event.clone())
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                Symbol::new("start"),
                Symbol::new("delta"),
                Symbol::new("delta"),
                Symbol::new("usage"),
                Symbol::new("final"),
            ]
        );
        assert!(format!("{:?}", events[1].extra).contains("one"));
        assert!(format!("{:?}", events[2].extra).contains("two"));
        assert!(format!("{:?}", response.content).contains("one"));
        assert!(format!("{:?}", response.content).contains("two"));
    }
}
