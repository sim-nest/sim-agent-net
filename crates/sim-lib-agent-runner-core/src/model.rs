use crate::{ModelBid, ModelCard, ModelEvent, ModelEventSink};
use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{Cx, EvalRequest, Expr, Result, Symbol};

/// Executable contract for a model backend or model-like runtime surface.
pub trait ModelRunner: Send + Sync {
    /// Returns metadata describing the runner's advertised model surface.
    fn card(&self) -> ModelCard;

    /// Executes one non-streaming inference request.
    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse>;

    /// Executes one non-streaming inference request with its full eval envelope.
    ///
    /// Implementations that need the caller's capability ceiling can override
    /// this method. The default keeps existing model-only runners compatible.
    fn infer_request(&self, cx: &mut Cx, request: EvalRequest) -> Result<ModelResponse> {
        self.infer(cx, ModelRequest::try_from(request.expr)?)
    }

    /// Executes one streaming inference request.
    ///
    /// The default implementation falls back to [`Self::infer`] and emits a
    /// synthesized final event.
    fn infer_stream(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        let response = self.infer(cx, request)?;
        sink.emit(ModelEvent::final_of(&response))?;
        Ok(response)
    }

    /// Executes one streaming inference request with its full eval envelope.
    ///
    /// The default preserves existing streaming behavior after decoding the
    /// model request from the envelope.
    fn infer_stream_request(
        &self,
        cx: &mut Cx,
        request: EvalRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        self.infer_stream(cx, ModelRequest::try_from(request.expr)?, sink)
    }

    /// Returns an availability and score hint for market-style routing.
    fn bid(&self, _request: &ModelRequest) -> Result<ModelBid> {
        Ok(ModelBid::unavailable("runner did not implement bidding"))
    }
}

/// Normalized model request used across providers and local runners.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "agent-runner/ModelRequest", version = 1)]
pub struct ModelRequest {
    /// Primary task expression, typically a transcript or chat request object.
    pub task: Expr,
    /// Prior message or context transcript items.
    pub messages: Vec<Expr>,
    /// Open extension fields preserved for runner-specific features.
    pub extra: Vec<(Expr, Expr)>,
}

impl ModelRequest {
    /// Builds a request from a task expression and message transcript.
    pub fn new(task: Expr, messages: Vec<Expr>) -> Self {
        Self {
            task,
            messages,
            extra: Vec::new(),
        }
    }
}

impl Default for ModelRequest {
    fn default() -> Self {
        Self::new(Expr::String("citizen fixture task".to_owned()), Vec::new())
    }
}

/// Final response returned by a runner after inference completes.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "agent-runner/ModelResponse", version = 1)]
pub struct ModelResponse {
    /// Runner identity that produced the response.
    pub runner: Symbol,
    /// Provider or runtime model name.
    pub model: String,
    /// Structured output content.
    pub content: Vec<Expr>,
    /// Stop reason symbol reported by the runner.
    pub stop_reason: Symbol,
    /// Optional usage and accounting metadata.
    pub usage: Option<ModelUsage>,
    /// Open extension fields preserved for runner-specific features.
    pub extra: Vec<(Expr, Expr)>,
}

impl ModelResponse {
    /// Constructs a response with explicit runner identity and stop reason.
    pub fn new(
        runner: Symbol,
        model: impl Into<String>,
        content: Vec<Expr>,
        stop_reason: Symbol,
    ) -> Self {
        Self {
            runner,
            model: model.into(),
            content,
            stop_reason,
            usage: None,
            extra: Vec::new(),
        }
    }
}

impl Default for ModelResponse {
    fn default() -> Self {
        Self::new(
            Symbol::qualified("runner", "fixture"),
            "fixture-model",
            vec![Expr::String("ok".to_owned())],
            Symbol::new("stop"),
        )
    }
}

/// Usage and accounting information attached to a model response.
#[derive(Clone, Debug, PartialEq, Default, Citizen)]
#[citizen(symbol = "agent-runner/ModelUsage", version = 1)]
pub struct ModelUsage {
    /// Count of input tokens when reported by the backend.
    pub input_tokens: Option<u64>,
    /// Count of output tokens when reported by the backend.
    pub output_tokens: Option<u64>,
    /// End-to-end latency in milliseconds when available.
    pub latency_ms: Option<u64>,
    /// Monetary cost estimate in USD when available.
    pub cost_usd: Option<f64>,
    /// Open extension fields preserved for runner-specific accounting data.
    pub extra: Vec<(Expr, Expr)>,
}

impl CitizenField for ModelUsage {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.input_tokens.encode_field(),
            self.output_tokens.encode_field(),
            self.latency_ms.encode_field(),
            self.cost_usd.encode_field(),
            self.extra.encode_field(),
        ])
    }

    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(items) = expr else {
            return Err(sim_citizen::field_error(field, "expected model usage list"));
        };
        let [input_tokens, output_tokens, latency_ms, cost_usd, extra] = items.as_slice() else {
            return Err(sim_citizen::field_error(
                field,
                format!("expected 5 model usage field(s), found {}", items.len()),
            ));
        };
        Ok(Self {
            input_tokens: Option::<u64>::decode_field_expr(input_tokens, field)?,
            output_tokens: Option::<u64>::decode_field_expr(output_tokens, field)?,
            latency_ms: Option::<u64>::decode_field_expr(latency_ms, field)?,
            cost_usd: Option::<f64>::decode_field_expr(cost_usd, field)?,
            extra: Vec::<(Expr, Expr)>::decode_field_expr(extra, field)?,
        })
    }
}

/// Returns the citizen class symbol for [`ModelRequest`].
pub fn model_request_class_symbol() -> Symbol {
    Symbol::qualified("agent-runner", "ModelRequest")
}

/// Returns the citizen class symbol for [`ModelResponse`].
pub fn model_response_class_symbol() -> Symbol {
    Symbol::qualified("agent-runner", "ModelResponse")
}

/// Returns the citizen class symbol for [`ModelUsage`].
pub fn model_usage_class_symbol() -> Symbol {
    Symbol::qualified("agent-runner", "ModelUsage")
}
