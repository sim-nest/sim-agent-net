use sim_codec_chat::decode_anthropic_stream_events;
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelEvent, ModelEventSink, ModelResponse};

pub(crate) struct AnthropicStreamDecoder {
    runner: Symbol,
    model: String,
    span_id: Expr,
    body: Vec<u8>,
    include_raw: bool,
}

impl AnthropicStreamDecoder {
    pub(super) fn new(runner: Symbol, model: String, include_raw: bool) -> Self {
        Self {
            runner,
            model,
            span_id: Expr::String("anthropic-sse".to_owned()),
            body: Vec::new(),
            include_raw,
        }
    }

    pub(super) fn start_event(&self) -> ModelEvent {
        ModelEvent::start(
            self.runner.clone(),
            self.model.clone(),
            self.span_id.clone(),
        )
    }

    pub(super) fn feed(&mut self, bytes: &[u8], _sink: &mut dyn ModelEventSink) -> Result<()> {
        self.body.extend_from_slice(bytes);
        Ok(())
    }

    pub(super) fn has_stream_output(&self) -> bool {
        !self.body.is_empty()
    }

    pub(super) fn finish(self, sink: &mut dyn ModelEventSink) -> Result<ModelResponse> {
        let events = decode_anthropic_stream_events(
            self.runner.clone(),
            &self.model,
            &self.body,
            self.include_raw,
        )?;
        let mut final_response = None;
        for expr in events {
            let event = ModelEvent::try_from(expr)?;
            if event.event == Symbol::new("start") {
                continue;
            }
            if event.event == Symbol::new("final") {
                final_response = event.response;
                continue;
            }
            sink.emit(event)?;
        }
        final_response.ok_or_else(|| {
            Error::Eval("anthropic stream did not produce a final response".to_owned())
        })
    }
}
