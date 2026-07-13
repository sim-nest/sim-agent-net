use serde_json::Value;
mod anthropic;

use anthropic::AnthropicStreamDecoder;
use sim_codec_chat::{number_field, text_part};
use sim_codec_json::{JsonProjectionMode, json_number_to_u64, project_json_to_expr};
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelResponse, ModelUsage};
use sim_lib_net_core::{LineDecoder, SseEvent};

pub(crate) enum HttpStreamDecoder {
    OpenAi(OpenAiStreamDecoder),
    Anthropic(AnthropicStreamDecoder),
    Ollama(OllamaStreamDecoder),
}

impl HttpStreamDecoder {
    pub(crate) fn openai(runner: Symbol, model: String, include_raw: bool) -> Self {
        Self::OpenAi(OpenAiStreamDecoder::new(runner, model, include_raw))
    }

    pub(crate) fn anthropic(runner: Symbol, model: String, include_raw: bool) -> Self {
        Self::Anthropic(AnthropicStreamDecoder::new(runner, model, include_raw))
    }

    pub(crate) fn ollama(runner: Symbol, model: String, include_raw: bool) -> Self {
        Self::Ollama(OllamaStreamDecoder::new(runner, model, include_raw))
    }

    pub(crate) fn start_event(&self) -> ModelEvent {
        match self {
            Self::OpenAi(decoder) => decoder.start_event(),
            Self::Anthropic(decoder) => decoder.start_event(),
            Self::Ollama(decoder) => decoder.start_event(),
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8], sink: &mut dyn ModelEventSink) -> Result<()> {
        match self {
            Self::OpenAi(decoder) => decoder.feed(bytes, sink),
            Self::Anthropic(decoder) => decoder.feed(bytes, sink),
            Self::Ollama(decoder) => decoder.feed(bytes, sink),
        }
    }

    pub(crate) fn has_stream_output(&self) -> bool {
        match self {
            Self::OpenAi(decoder) => decoder.has_stream_output(),
            Self::Anthropic(decoder) => decoder.has_stream_output(),
            Self::Ollama(decoder) => decoder.has_stream_output(),
        }
    }

    pub(crate) fn finish(self, sink: &mut dyn ModelEventSink) -> Result<ModelResponse> {
        match self {
            Self::OpenAi(decoder) => decoder.finish(sink),
            Self::Anthropic(decoder) => decoder.finish(sink),
            Self::Ollama(decoder) => decoder.finish(sink),
        }
    }
}

pub(crate) struct OpenAiStreamDecoder {
    runner: Symbol,
    model: String,
    span_id: Expr,
    lines: LineDecoder,
    combined: String,
    usage: Option<ModelUsage>,
    stop_reason: Symbol,
    raw_chunks: Vec<Expr>,
    include_raw: bool,
    provider_chunks: usize,
}

impl OpenAiStreamDecoder {
    fn new(runner: Symbol, model: String, include_raw: bool) -> Self {
        Self {
            runner,
            model,
            span_id: Expr::String("openai-sse".to_owned()),
            lines: LineDecoder::new(),
            combined: String::new(),
            usage: None,
            stop_reason: Symbol::new("stop"),
            raw_chunks: Vec::new(),
            include_raw,
            provider_chunks: 0,
        }
    }

    fn start_event(&self) -> ModelEvent {
        ModelEvent::start(
            self.runner.clone(),
            self.model.clone(),
            self.span_id.clone(),
        )
    }

    fn feed(&mut self, bytes: &[u8], sink: &mut dyn ModelEventSink) -> Result<()> {
        for line in self.lines.push(bytes) {
            self.consume_line(&line, sink)?;
        }
        Ok(())
    }

    fn consume_line(&mut self, line: &[u8], sink: &mut dyn ModelEventSink) -> Result<()> {
        let line = std::str::from_utf8(line)
            .map_err(|err| Error::Eval(format!("openai stream is not valid utf-8: {err}")))?;
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            return Ok(());
        }
        let Some(payload) = line.strip_prefix("data:") else {
            return Ok(());
        };
        // Each non-blank `data:` line is dispatched as its own SSE record,
        // preserving the original per-line (not blank-line-batched) semantics.
        let event = SseEvent {
            event: None,
            data: payload.trim().to_owned(),
        };
        if event.data == "[DONE]" {
            return Ok(());
        }
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|err| Error::Eval(format!("openai stream returned invalid json: {err}")))?;
        self.provider_chunks += 1;
        if self.include_raw {
            self.raw_chunks.push(project_json_to_expr(
                &value,
                JsonProjectionMode::UntaggedInterop,
            ));
        }
        if let Some(usage) = openai_usage(&value) {
            self.usage = Some(usage.clone());
            sink.emit(ModelEvent::usage(
                self.runner.clone(),
                self.model.clone(),
                self.span_id.clone(),
                usage,
            ))?;
        }
        if let Some(text) = openai_delta_text(&value)? {
            self.combined.push_str(&text);
            sink.emit(ModelEvent::delta_text(
                self.runner.clone(),
                self.model.clone(),
                self.span_id.clone(),
                text,
            ))?;
        }
        if let Some(reason) = openai_finish_reason(&value) {
            self.stop_reason = Symbol::new(reason);
        }
        Ok(())
    }

    fn has_stream_output(&self) -> bool {
        self.provider_chunks > 0 || self.has_pending_stream_line()
    }

    fn has_pending_stream_line(&self) -> bool {
        std::str::from_utf8(self.lines.buffered())
            .map(|line| line.trim_start().starts_with("data:"))
            .unwrap_or(false)
    }

    fn finish(mut self, sink: &mut dyn ModelEventSink) -> Result<ModelResponse> {
        if let Some(line) = self.lines.flush() {
            self.consume_line(&line, sink)?;
        }
        let mut response = ModelResponse::new(
            self.runner,
            self.model,
            vec![text_part(&self.combined)],
            self.stop_reason,
        );
        response.usage = self.usage;
        if self.include_raw {
            response.extra.push((
                Expr::Symbol(Symbol::new("raw-provider-response")),
                Expr::List(self.raw_chunks),
            ));
        }
        Ok(response)
    }
}

pub(crate) struct OllamaStreamDecoder {
    runner: Symbol,
    model: String,
    span_id: Expr,
    lines: LineDecoder,
    combined: String,
    usage: Option<ModelUsage>,
    stop_reason: Symbol,
    raw_chunks: Vec<Expr>,
    include_raw: bool,
    provider_chunks: usize,
}

impl OllamaStreamDecoder {
    fn new(runner: Symbol, model: String, include_raw: bool) -> Self {
        Self {
            runner,
            model,
            span_id: Expr::String("ollama-ndjson".to_owned()),
            lines: LineDecoder::new(),
            combined: String::new(),
            usage: None,
            stop_reason: Symbol::new("stop"),
            raw_chunks: Vec::new(),
            include_raw,
            provider_chunks: 0,
        }
    }

    fn start_event(&self) -> ModelEvent {
        ModelEvent::start(
            self.runner.clone(),
            self.model.clone(),
            self.span_id.clone(),
        )
    }

    fn feed(&mut self, bytes: &[u8], sink: &mut dyn ModelEventSink) -> Result<()> {
        for line in self.lines.push(bytes) {
            self.consume_line(&line, sink)?;
        }
        Ok(())
    }

    fn consume_line(&mut self, line: &[u8], sink: &mut dyn ModelEventSink) -> Result<()> {
        let line = std::str::from_utf8(line)
            .map_err(|err| Error::Eval(format!("ollama stream is not valid utf-8: {err}")))?;
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|err| Error::Eval(format!("ollama stream returned invalid json: {err}")))?;
        self.provider_chunks += 1;
        if self.include_raw {
            self.raw_chunks.push(project_json_to_expr(
                &value,
                JsonProjectionMode::UntaggedInterop,
            ));
        }
        if let Some(text) = ollama_chunk_text(&value)? {
            self.combined.push_str(&text);
            sink.emit(ModelEvent::delta_text(
                self.runner.clone(),
                self.model.clone(),
                self.span_id.clone(),
                text,
            ))?;
        }
        if let Some(usage) = ollama_usage(&value) {
            self.usage = Some(usage.clone());
            sink.emit(ModelEvent::usage(
                self.runner.clone(),
                self.model.clone(),
                self.span_id.clone(),
                usage,
            ))?;
        }
        if let Some(reason) = value.get("done_reason").and_then(Value::as_str) {
            self.stop_reason = Symbol::new(reason);
        } else if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
            self.stop_reason = Symbol::new("stop");
        }
        Ok(())
    }

    fn has_stream_output(&self) -> bool {
        self.provider_chunks > 0 || self.lines.has_buffered()
    }

    fn finish(mut self, sink: &mut dyn ModelEventSink) -> Result<ModelResponse> {
        if let Some(line) = self.lines.flush() {
            self.consume_line(&line, sink)?;
        }
        let mut response = ModelResponse::new(
            self.runner,
            self.model,
            vec![text_part(&self.combined)],
            self.stop_reason,
        );
        response.usage = self.usage;
        if self.include_raw {
            response.extra.push((
                Expr::Symbol(Symbol::new("raw-provider-response")),
                Expr::List(self.raw_chunks),
            ));
        }
        Ok(response)
    }
}

fn openai_delta_text(value: &Value) -> Result<Option<String>> {
    let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
        return Ok(None);
    };
    match delta.get("content") {
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        Some(Value::Array(parts)) => Ok(Some(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
        )
        .filter(|text| !text.is_empty())),
        Some(Value::Null) | None => Ok(None),
        _ => Err(Error::Eval(
            "openai stream delta content must be string, array, or null".to_owned(),
        )),
    }
}

fn openai_finish_reason(value: &Value) -> Option<String> {
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn openai_usage(value: &Value) -> Option<ModelUsage> {
    let usage = value.get("usage").and_then(Value::as_object)?;
    let mut model_usage = ModelUsage {
        input_tokens: usage.get("prompt_tokens").and_then(json_number_to_u64),
        output_tokens: usage.get("completion_tokens").and_then(json_number_to_u64),
        ..ModelUsage::default()
    };
    if let Some(total) = usage.get("total_tokens").and_then(json_number_to_u64) {
        model_usage.extra.push(number_field("total-tokens", total));
    }
    Some(model_usage)
}

fn ollama_chunk_text(value: &Value) -> Result<Option<String>> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::Eval("ollama stream chunk must be a json object".to_owned()))?;
    if let Some(content) = object
        .get("message")
        .and_then(Value::as_object)
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
    {
        return Ok(Some(content.to_owned()));
    }
    Ok(object
        .get("response")
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .map(str::to_owned))
}

fn ollama_usage(value: &Value) -> Option<ModelUsage> {
    let input_tokens = value.get("prompt_eval_count").and_then(json_number_to_u64);
    let output_tokens = value.get("eval_count").and_then(json_number_to_u64);
    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }
    let mut usage = ModelUsage {
        input_tokens,
        output_tokens,
        ..ModelUsage::default()
    };
    if let (Some(input), Some(output)) = (input_tokens, output_tokens) {
        usage
            .extra
            .push(number_field("total-tokens", input + output));
    }
    Some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CollectEvents {
        events: Vec<ModelEvent>,
    }

    impl ModelEventSink for CollectEvents {
        fn emit(&mut self, event: ModelEvent) -> Result<()> {
            self.events.push(event);
            Ok(())
        }
    }

    #[test]
    fn openai_finish_emits_unterminated_delta_to_sink() {
        let mut decoder =
            HttpStreamDecoder::openai(Symbol::new("http-openai"), "gpt-test".to_owned(), true);
        let mut sink = CollectEvents::default();
        decoder
            .feed(
                br#"data: {"choices":[{"index":0,"delta":{"content":"tail"},"finish_reason":null}]}"#,
                &mut sink,
            )
            .unwrap();
        assert!(sink.events.is_empty());
        assert!(decoder.has_stream_output());

        let response = decoder.finish(&mut sink).unwrap();

        assert_eq!(sink.events.len(), 1);
        assert_eq!(sink.events[0].event, Symbol::new("delta"));
        assert_eq!(
            sink.events[0].extra,
            vec![(
                Expr::Symbol(Symbol::new("text")),
                Expr::String("tail".to_owned()),
            )]
        );
        assert!(format!("{:?}", response.content).contains("tail"));
        assert_eq!(response.extra.len(), 1);
    }

    #[test]
    fn ollama_finish_emits_unterminated_delta_to_sink() {
        let mut decoder =
            HttpStreamDecoder::ollama(Symbol::new("http-ollama"), "qwen-test".to_owned(), false);
        let mut sink = CollectEvents::default();
        decoder
            .feed(
                br#"{"model":"qwen-test","message":{"role":"assistant","content":"tail"},"done":false}"#,
                &mut sink,
            )
            .unwrap();
        assert!(sink.events.is_empty());
        assert!(decoder.has_stream_output());

        let response = decoder.finish(&mut sink).unwrap();

        assert_eq!(sink.events.len(), 1);
        assert_eq!(sink.events[0].event, Symbol::new("delta"));
        assert!(format!("{:?}", sink.events[0].extra).contains("tail"));
        assert!(format!("{:?}", response.content).contains("tail"));
    }
}
