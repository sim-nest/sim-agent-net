use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use sim_citizen_derive::non_citizen;
use sim_codec_mcp::{McpEnvelope, McpRequest, McpResponse};
use sim_kernel::{CapabilityName, Cx, Error, Expr, Object, ObjectCompat, Result, Symbol, Value};
use sim_lib_agent_runner_core::{
    ModelBid, ModelCard, ModelEvent, ModelEventSink, ModelRequest, ModelResponse, ModelRunner,
};

/// Returns the capability gating MCP sampling requests.
pub fn mcp_sampling_capability() -> CapabilityName {
    CapabilityName::new("mcp.sampling")
}

/// Returns the runtime symbol naming the MCP sampling runner.
pub fn mcp_sampling_runner_symbol() -> Symbol {
    Symbol::qualified("mcp", "sampling-runner")
}

/// Returns the stream data kind tagging MCP sampling stream packets.
pub fn mcp_sampling_data_kind() -> Symbol {
    Symbol::qualified("stream/data", "mcp-sampling")
}

/// Host that fulfills `sampling/createMessage` exchanges for a runner.
pub trait McpSamplingHost: Send + Sync {
    /// Sends `envelope` to the sampling host and returns its reply.
    fn exchange(&self, cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope>;
}

/// [`ModelRunner`] that performs inference via MCP `sampling/createMessage`.
pub struct McpSamplingRunner {
    runner: Symbol,
    model: String,
    host: Arc<dyn McpSamplingHost>,
    next_id: AtomicU64,
    #[cfg(feature = "cassette")]
    cassette: Option<Arc<Mutex<crate::McpCassette>>>,
}

impl McpSamplingRunner {
    /// Creates a runner named `runner` for `model` backed by `host`.
    pub fn new(runner: Symbol, model: impl Into<String>, host: Arc<dyn McpSamplingHost>) -> Self {
        Self {
            runner,
            model: model.into(),
            host,
            next_id: AtomicU64::new(1),
            #[cfg(feature = "cassette")]
            cassette: None,
        }
    }

    /// Creates a runner backed by a canned [`FixtureSamplingHost`].
    pub fn fixture() -> Self {
        Self::new(
            Symbol::qualified("mcp", "sampling-fixture"),
            "mcp/sampling-fixture",
            Arc::new(FixtureSamplingHost::text("fixture sampling response")),
        )
    }

    /// Returns the runner with a shared record/replay `cassette` attached.
    #[cfg(feature = "cassette")]
    pub fn with_cassette(mut self, cassette: Arc<Mutex<crate::McpCassette>>) -> Self {
        self.cassette = Some(cassette);
        self
    }

    fn request(&self, cx: &mut Cx, request: ModelRequest) -> Result<Expr> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let envelope = McpEnvelope::Request(McpRequest {
            id: Expr::String(format!("mcp-sampling-{id}")),
            method: "sampling/createMessage".to_owned(),
            params: sampling_params(&self.model, request),
        });
        match self.exchange(cx, envelope)? {
            McpEnvelope::Response(McpResponse { result, .. }) => Ok(result),
            McpEnvelope::Error(error) => Err(Error::Eval(format!(
                "foreign MCP sampling error {}: {}",
                error.error.code, error.error.message
            ))),
            _ => Err(Error::Eval(
                "foreign MCP sampling host returned non-response".to_owned(),
            )),
        }
    }

    fn exchange(&self, cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope> {
        #[cfg(feature = "cassette")]
        if let Some(cassette) = &self.cassette {
            let mut cassette = cassette
                .lock()
                .map_err(|_| Error::PoisonedLock("mcp sampling cassette"))?;
            if let Some(mut replies) = cassette.replay(&envelope)? {
                cassette.record_audit("sampling/createMessage", "sampling", "replay");
                if replies.len() == 1 {
                    return Ok(replies.remove(0));
                }
                return Err(Error::Eval(
                    "MCP sampling cassette replay expected one reply".to_owned(),
                ));
            }
        }
        let reply = self.host.exchange(cx, envelope.clone())?;
        #[cfg(feature = "cassette")]
        if let Some(cassette) = &self.cassette {
            let mut cassette = cassette
                .lock()
                .map_err(|_| Error::PoisonedLock("mcp sampling cassette"))?;
            cassette.record_exchange(&envelope, std::slice::from_ref(&reply))?;
            cassette.record_audit(
                "sampling/createMessage",
                "sampling",
                if matches!(reply, McpEnvelope::Error(_)) {
                    "error"
                } else {
                    "ok"
                },
            );
        }
        Ok(reply)
    }

    fn response_from_result(&self, result: &Expr) -> Result<ModelResponse> {
        if let Some(response) = optional_field(result, "response") {
            return self.normalize_response(response.clone());
        }
        self.normalize_response(result.clone()).or_else(|_| {
            let content = response_content(result);
            Ok(ModelResponse::new(
                self.runner.clone(),
                self.model.clone(),
                content,
                Symbol::new("stop"),
            ))
        })
    }

    fn normalize_response(&self, expr: Expr) -> Result<ModelResponse> {
        let mut response = ModelResponse::try_from(expr)?;
        response.runner = self.runner.clone();
        response.model = self.model.clone();
        Ok(response)
    }
}

impl ModelRunner for McpSamplingRunner {
    fn card(&self) -> ModelCard {
        let mut card = ModelCard::new(
            self.runner.clone(),
            self.model.clone(),
            Symbol::new("mcp"),
            Symbol::new("remote"),
        );
        card.extra.extend([
            field("method", Expr::String("sampling/createMessage".to_owned())),
            field(
                "capability",
                Expr::String(mcp_sampling_capability().to_string()),
            ),
            field("supports-stream", Expr::Bool(true)),
        ]);
        card
    }

    fn infer(&self, cx: &mut Cx, request: ModelRequest) -> Result<ModelResponse> {
        cx.require(&mcp_sampling_capability())?;
        let result = self.request(cx, request)?;
        self.response_from_result(&result)
    }

    fn infer_stream(
        &self,
        cx: &mut Cx,
        request: ModelRequest,
        sink: &mut dyn ModelEventSink,
    ) -> Result<ModelResponse> {
        cx.require(&mcp_sampling_capability())?;
        let result = self.request(cx, request)?;
        let span = Expr::String("mcp-sampling".to_owned());
        sink.emit(ModelEvent::start(
            self.runner.clone(),
            self.model.clone(),
            span.clone(),
        ))?;
        let mut saw_final = false;
        for event in sampling_events(&result) {
            let event = event_from_sampling_expr(&self.runner, &self.model, &span, event)?;
            #[cfg(feature = "stream")]
            let event = {
                let packet_payload = Expr::from(event.clone());
                event.with_field(
                    "stream-packet",
                    sim_lib_stream_core::StreamPacket::data(
                        mcp_sampling_data_kind(),
                        packet_payload,
                    )
                    .to_expr(),
                )
            };
            if event.event == Symbol::new("final") {
                saw_final = true;
            }
            sink.emit(event)?;
        }
        let response = self.response_from_result(&result)?;
        if !saw_final {
            sink.emit(ModelEvent::final_of(&response))?;
        }
        Ok(response)
    }

    fn bid(&self, _request: &ModelRequest) -> Result<ModelBid> {
        Ok(ModelBid {
            available: true,
            reason: None,
            score: Some(0.0),
            model: Some(self.model.clone()),
            extra: vec![field(
                "method",
                Expr::String("sampling/createMessage".to_owned()),
            )],
        })
    }
}

#[non_citizen(
    reason = "live MCP sampling host bridge; requests and responses use mcp/Request and mcp/Response descriptors",
    kind = "handle",
    descriptor = "mcp/Request"
)]
/// Runtime object wrapper exposing an [`McpSamplingRunner`] as a SIM value.
pub struct McpSamplingRunnerValue {
    runner: Arc<McpSamplingRunner>,
}

impl McpSamplingRunnerValue {
    /// Wraps `runner` as a runtime object.
    pub fn new(runner: Arc<McpSamplingRunner>) -> Self {
        Self { runner }
    }

    /// Returns the wrapped runner.
    pub fn runner(&self) -> Arc<McpSamplingRunner> {
        self.runner.clone()
    }
}

impl Object for McpSamplingRunnerValue {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<mcp-sampling-runner {}>", self.runner.model))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for McpSamplingRunnerValue {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::from(self.runner.card()))
    }
}

/// Wraps `runner` in an opaque runtime [`Value`].
pub fn sampling_runner_value(cx: &mut Cx, runner: Arc<McpSamplingRunner>) -> Result<Value> {
    cx.factory()
        .opaque(Arc::new(McpSamplingRunnerValue::new(runner)))
}

/// In-memory [`McpSamplingHost`] that returns a canned result for tests.
pub struct FixtureSamplingHost {
    result: Mutex<Expr>,
    calls: AtomicUsize,
    methods: Mutex<Vec<String>>,
}

impl FixtureSamplingHost {
    /// Creates a host that always replies with `result`.
    pub fn new(result: Expr) -> Self {
        Self {
            result: Mutex::new(result),
            calls: AtomicUsize::new(0),
            methods: Mutex::new(Vec::new()),
        }
    }

    /// Creates a host that replies with a single text content part.
    pub fn text(text: impl Into<String>) -> Self {
        Self::new(sampling_text_result(text.into()))
    }

    /// Creates a host that replies with streamed `deltas` then `final_text`.
    pub fn streamed(deltas: Vec<String>, final_text: impl Into<String>) -> Self {
        let events = deltas
            .into_iter()
            .map(|text| Expr::Map(vec![field("text", Expr::String(text))]))
            .collect();
        Self::new(Expr::Map(vec![
            field("events", Expr::List(events)),
            field("response", Expr::from(model_response(final_text.into()))),
        ]))
    }

    /// Returns the number of exchanges handled so far.
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Returns the methods of the requests handled so far.
    pub fn methods(&self) -> Result<Vec<String>> {
        Ok(self
            .methods
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture sampling methods"))?
            .clone())
    }
}

impl McpSamplingHost for FixtureSamplingHost {
    fn exchange(&self, _cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope> {
        let McpEnvelope::Request(request) = envelope else {
            return Err(Error::TypeMismatch {
                expected: "MCP sampling request",
                found: "non-request",
            });
        };
        self.methods
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture sampling methods"))?
            .push(request.method);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(McpEnvelope::Response(McpResponse {
            id: request.id,
            result: self
                .result
                .lock()
                .map_err(|_| Error::PoisonedLock("fixture sampling result"))?
                .clone(),
        }))
    }
}

fn sampling_params(model: &str, request: ModelRequest) -> Expr {
    let Expr::Map(mut fields) = Expr::from(request) else {
        unreachable!("ModelRequest always converts to a map")
    };
    fields.insert(0, field("model", Expr::String(model.to_owned())));
    Expr::Map(fields)
}

fn response_content(result: &Expr) -> Vec<Expr> {
    match optional_field(result, "content") {
        Some(Expr::List(items)) if !items.is_empty() => items.clone(),
        Some(Expr::String(text)) => vec![text_part(text.clone())],
        _ => vec![text_part(expr_text(result))],
    }
}

fn sampling_events(result: &Expr) -> &[Expr] {
    match optional_field(result, "events") {
        Some(Expr::List(items)) => items,
        _ => &[],
    }
}

fn event_from_sampling_expr(
    runner: &Symbol,
    model: &str,
    span: &Expr,
    expr: &Expr,
) -> Result<ModelEvent> {
    if let Ok(mut event) = ModelEvent::try_from(expr.clone()) {
        event.runner = runner.clone();
        event.model = model.to_owned();
        return Ok(event);
    }
    let text = optional_field(expr, "text")
        .and_then(|value| match value {
            Expr::String(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| expr_text(expr));
    Ok(ModelEvent::delta_text(
        runner.clone(),
        model.to_owned(),
        span.clone(),
        text,
    ))
}

fn sampling_text_result(text: String) -> Expr {
    Expr::Map(vec![field("content", Expr::List(vec![text_part(text)]))])
}

fn model_response(text: String) -> ModelResponse {
    ModelResponse::new(
        Symbol::qualified("mcp", "fixture-host"),
        "fixture-host",
        vec![text_part(text)],
        Symbol::new("stop"),
    )
}

// Canonical shape lives in `sim_codec_chat::text_part`. Kept local here to avoid
// coupling `sim-lib-mcp` to the chat codec for one trivial content-part builder.
fn text_part(text: String) -> Expr {
    Expr::Map(vec![
        field("type", Expr::Symbol(Symbol::new("text"))),
        field("text", Expr::String(text)),
    ])
}

fn expr_text(expr: &Expr) -> String {
    match expr {
        Expr::Nil => String::new(),
        Expr::Bool(value) => value.to_string(),
        Expr::Number(number) => number.canonical.clone(),
        Expr::String(value) => value.clone(),
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol.as_qualified_str(),
        other => format!("{other:?}"),
    }
}

use sim_value::access::field_any as optional_field;
use sim_value::build::entry as field;
