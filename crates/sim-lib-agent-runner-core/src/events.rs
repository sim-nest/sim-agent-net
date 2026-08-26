use crate::{ModelRequest, ModelResponse, ModelUsage};
use sim_kernel::{Cx, Datum, DatumStore, Event, EventKind, Expr, Ref, Result, Symbol};

/// Sink for streaming model events emitted during inference.
pub trait ModelEventSink {
    /// Receives one event from the active runner invocation.
    fn emit(&mut self, event: ModelEvent) -> Result<()>;
}

/// In-memory event sink used by tests and buffered callers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VecEventSink {
    events: Vec<ModelEvent>,
}

impl VecEventSink {
    /// Constructs an empty event sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes the sink and returns all collected events.
    pub fn into_events(self) -> Vec<ModelEvent> {
        self.events
    }

    /// Borrows the collected events.
    pub fn events(&self) -> &[ModelEvent] {
        &self.events
    }
}

impl ModelEventSink for VecEventSink {
    fn emit(&mut self, event: ModelEvent) -> Result<()> {
        self.events.push(event);
        Ok(())
    }
}

/// Structured streaming event emitted by a model runner.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelEvent {
    /// Event kind such as `start`, `delta`, `usage`, `tool-call`, or `final`.
    pub event: Symbol,
    /// Runner identity that produced the event.
    pub runner: Symbol,
    /// Provider or runtime model name for the event.
    pub model: String,
    /// Correlation id for one inference span.
    pub span_id: Expr,
    /// Final response payload for terminal events when available.
    pub response: Option<ModelResponse>,
    /// Open extension fields preserved for event-specific payloads.
    pub extra: Vec<(Expr, Expr)>,
}

impl ModelEvent {
    /// Constructs a generic event record.
    pub fn new(event: Symbol, runner: Symbol, model: impl Into<String>, span_id: Expr) -> Self {
        Self {
            event,
            runner,
            model: model.into(),
            span_id,
            response: None,
            extra: Vec::new(),
        }
    }

    /// Constructs a `start` event.
    pub fn start(runner: Symbol, model: impl Into<String>, span_id: Expr) -> Self {
        Self::new(Symbol::new("start"), runner, model, span_id)
    }

    /// Constructs a text delta event.
    pub fn delta_text(
        runner: Symbol,
        model: impl Into<String>,
        span_id: Expr,
        text: impl Into<String>,
    ) -> Self {
        Self::new(Symbol::new("delta"), runner, model, span_id).with_text(text)
    }

    /// Constructs a usage event.
    pub fn usage(
        runner: Symbol,
        model: impl Into<String>,
        span_id: Expr,
        usage: ModelUsage,
    ) -> Self {
        Self::new(Symbol::new("usage"), runner, model, span_id)
            .with_field("usage", Expr::from(usage))
    }

    /// Constructs an error event with a text payload.
    pub fn error_text(
        runner: Symbol,
        model: impl Into<String>,
        span_id: Expr,
        message: impl Into<String>,
    ) -> Self {
        Self::new(Symbol::new("error"), runner, model, span_id).with_text(message)
    }

    /// Constructs a tool-call event.
    pub fn tool_call(
        runner: Symbol,
        model: impl Into<String>,
        span_id: Expr,
        tool_call: Expr,
    ) -> Self {
        Self::new(Symbol::new("tool-call"), runner, model, span_id)
            .with_field("tool-call", tool_call)
    }

    /// Constructs a terminal `final` event from a completed response.
    pub fn final_of(response: &ModelResponse) -> Self {
        Self {
            event: Symbol::new("final"),
            runner: response.runner.clone(),
            model: response.model.clone(),
            span_id: Expr::Symbol(Symbol::new("final")),
            response: Some(response.clone()),
            extra: Vec::new(),
        }
    }

    /// Appends an extra field to the event payload.
    pub fn with_field(mut self, key: &str, value: Expr) -> Self {
        self.extra.push((Expr::Symbol(Symbol::new(key)), value));
        self
    }

    fn with_text(self, text: impl Into<String>) -> Self {
        self.with_field("text", Expr::String(text.into()))
    }

    /// Converts this model event into a kernel chunk event with a content ref.
    pub fn to_kernel_event(&self, cx: &mut Cx, run: Ref, seq: u64) -> Result<Event> {
        model_expr_event(cx, run, seq, Expr::from(self.clone()))
    }
}

/// Converts a model request into a kernel chunk event with a content ref.
pub fn model_request_kernel_event(
    cx: &mut Cx,
    run: Ref,
    seq: u64,
    request: &ModelRequest,
) -> Result<Event> {
    model_expr_event(cx, run, seq, Expr::from(request.clone()))
}

fn model_expr_event(cx: &mut Cx, run: Ref, seq: u64, expr: Expr) -> Result<Event> {
    let payload = Ref::Content(cx.datum_store_mut().intern(Datum::try_from(expr)?)?);
    Event::new(run, seq, Vec::new(), EventKind::Chunk { payload })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::{DefaultFactory, NoopEvalPolicy};
    use std::sync::Arc;

    #[test]
    fn model_request_and_delta_convert_to_kernel_chunk_events() {
        let mut cx = Cx::new(
            Arc::new(NoopEvalPolicy),
            Arc::new(DefaultFactory),
            sim_kernel::HandleSeed::new(0xaa0a_e72c_c8f6_c03e),
        );
        let run = Ref::Symbol(Symbol::qualified("test", "run"));
        let request = ModelRequest::new(Expr::String("task".to_owned()), Vec::new());
        let request_event = model_request_kernel_event(&mut cx, run.clone(), 0, &request).unwrap();
        let delta = ModelEvent::delta_text(
            Symbol::qualified("runner", "fake"),
            "fake/model",
            Expr::String("span-1".to_owned()),
            "hel",
        );
        let delta_event = delta.to_kernel_event(&mut cx, run.clone(), 1).unwrap();

        assert!(matches!(request_event.kind, EventKind::Chunk { .. }));
        assert!(matches!(delta_event.kind, EventKind::Chunk { .. }));
        assert_eq!(delta_event.run, run);
        assert_eq!(delta_event.seq, 1);
    }
}
