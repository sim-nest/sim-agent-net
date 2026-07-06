//! Shared substrate for the gateway execution-record engines (OVERLAP9.04).
//!
//! The `/v1/responses`, `/v1/embeddings`, and run_record routes each drive the
//! same content-addressed ledger: mint a request id, redact and record the
//! request; open a run; append a sequence of events; store a final response.
//! Before OVERLAP9.04 that machinery was forked three ways (three id-generator
//! trios, three `EventLog`s, three `append_event` bodies, three request+run
//! prologues, three outcome structs). This module owns the ONE copy behind the
//! OVERLAP9.03 goldens.
//!
//! What stays route-local (and must, to keep the goldens byte-identical): the
//! object-vs-bare response storage split (`put_response_object` for responses
//! vs `put_response` for embeddings/run_record), the per-route event sequence
//! and kinds, and each route's `store` default.

use sim_kernel::{ContentId, Expr, Symbol};

use crate::{
    clock::GatewayClock,
    content_id::{content_id_for_expr, request_content_id},
    ids::GatewayIdGenerator,
    objects::{GatewayEvent, GatewayRequest, GatewayResponse, GatewayRun},
    runtime::redacted_gateway_request,
    storage::GatewayStore,
};

use super::errors::OpenAiRouteError;

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// Per-kind id generators for a single gateway run.
///
/// All three engines seed the request/run/event generators identically
/// (`gwreq`/`gwrun`/`gwevt` from the same start), so one struct serves them
/// all. The extra `response` generator (`resp`) is only advanced by the
/// `/v1/responses` engine, which mints a public response id; the other engines
/// simply never touch it.
#[derive(Clone, Debug)]
pub struct GatewayRunIdGenerators {
    request: GatewayIdGenerator,
    run: GatewayIdGenerator,
    /// Response id generator, used only by the `/v1/responses` engine.
    pub(crate) response: GatewayIdGenerator,
    event: GatewayIdGenerator,
}

impl GatewayRunIdGenerators {
    /// Builds id generators seeded deterministically from `start`, so a given
    /// seed always yields the same id sequence.
    pub fn deterministic(start: u64) -> Self {
        Self {
            request: GatewayIdGenerator::deterministic("gwreq", start),
            run: GatewayIdGenerator::deterministic("gwrun", start),
            response: GatewayIdGenerator::deterministic("resp", start),
            event: GatewayIdGenerator::deterministic("gwevt", start),
        }
    }

    pub(crate) fn next_event_id(&mut self) -> sim_kernel::Result<String> {
        self.event.next_id()
    }
}

/// One event appended to a run: its sequence, kind, and payload.
pub(crate) struct EventInput {
    sequence: u64,
    kind: Symbol,
    payload: Expr,
}

impl EventInput {
    pub(crate) fn new(sequence: u64, kind: &'static str, payload: Expr) -> Self {
        Self::from_symbol(sequence, Symbol::new(kind), payload)
    }

    pub(crate) fn from_symbol(sequence: u64, kind: Symbol, payload: Expr) -> Self {
        Self {
            sequence,
            kind,
            payload,
        }
    }
}

/// Accumulates the content ids and events produced while a run executes.
#[derive(Default)]
pub(crate) struct EventLog {
    pub(crate) content_ids: Vec<ContentId>,
    pub(crate) events: Vec<GatewayEvent>,
}

/// Mints an event id, builds the [`GatewayEvent`], records it under its content
/// id when `store_event` is set, and appends both to `event_log`.
pub(crate) fn append_event<S, C>(
    store: &mut S,
    ids: &mut GatewayRunIdGenerators,
    clock: &mut C,
    run_id: &str,
    input: EventInput,
    store_event: bool,
    event_log: &mut EventLog,
) -> RouteResult<()>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let event = GatewayEvent::new(
        ids.next_event_id().map_err(OpenAiRouteError::internal)?,
        run_id,
        input.sequence,
        input.kind,
        input.payload,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    let id = content_id_for_expr(&event.to_expr()).map_err(OpenAiRouteError::internal)?;
    if store_event {
        store
            .put_event(id.clone(), event.clone())
            .map_err(OpenAiRouteError::internal)?;
    }
    event_log.content_ids.push(id);
    event_log.events.push(event);
    Ok(())
}

/// The recorded request and open run produced by [`begin_run`].
pub(crate) struct RunPrologue {
    pub(crate) recorded_request: GatewayRequest,
    pub(crate) request_content_id: ContentId,
    pub(crate) run_id: String,
    pub(crate) run_content_id: ContentId,
}

/// Runs the shared request+run prologue: redact `request`, stamp it with a
/// fresh request id and timestamp, record it (when `store_record`), then open a
/// run pointing at it and record that too.
///
/// `run_status` is applied to the run before its content id is computed; the
/// run_record engine passes `completed`, while responses/embeddings pass
/// `None` and leave the run status at its default. Keeping this optional is
/// what preserves each route's run wire form.
pub(crate) fn begin_run<S, C>(
    store: &mut S,
    ids: &mut GatewayRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    run_status: Option<Symbol>,
    store_record: bool,
) -> RouteResult<RunPrologue>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let recorded_request = redacted_gateway_request(request).with_metadata(
        ids.request.next_id().map_err(OpenAiRouteError::internal)?,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    let request_content_id =
        request_content_id(&recorded_request).map_err(OpenAiRouteError::internal)?;
    if store_record {
        store
            .put_request(request_content_id.clone(), recorded_request.clone())
            .map_err(OpenAiRouteError::internal)?;
    }

    let run_id = ids.run.next_id().map_err(OpenAiRouteError::internal)?;
    let mut run = GatewayRun::new(
        run_id.clone(),
        request_content_id.clone(),
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    if let Some(status) = run_status {
        run = run.with_status(status);
    }
    let run_content_id = content_id_for_expr(&run.to_expr()).map_err(OpenAiRouteError::internal)?;
    if store_record {
        store
            .put_run(run_content_id.clone(), run)
            .map_err(OpenAiRouteError::internal)?;
    }

    Ok(RunPrologue {
        recorded_request,
        request_content_id,
        run_id,
        run_content_id,
    })
}

/// The outcome of executing a gateway run: the wire response plus the
/// content-addressed ledger ids and events it produced.
///
/// `response_id`/`response_created_at_ms` are only populated by the
/// `/v1/responses` engine (which mints a public response object); the other
/// engines leave them `None`.
#[derive(Clone, Debug)]
pub struct GatewayRunExecution {
    pub(crate) response: GatewayResponse,
    pub(crate) request_content_id: Option<ContentId>,
    pub(crate) run_content_id: Option<ContentId>,
    pub(crate) event_content_ids: Vec<ContentId>,
    pub(crate) events: Vec<GatewayEvent>,
    pub(crate) response_id: Option<String>,
    pub(crate) response_created_at_ms: Option<u64>,
    pub(crate) response_content_id: Option<ContentId>,
}

impl GatewayRunExecution {
    /// Returns the wire response produced by the execution.
    pub fn response(&self) -> &GatewayResponse {
        &self.response
    }

    /// Returns the content id of the stored request, if it was recorded.
    pub fn request_content_id(&self) -> Option<&ContentId> {
        self.request_content_id.as_ref()
    }

    /// Returns the content id of the stored run, if it was recorded.
    pub fn run_content_id(&self) -> Option<&ContentId> {
        self.run_content_id.as_ref()
    }

    /// Returns the content ids of the stored events, in sequence order.
    pub fn event_content_ids(&self) -> &[ContentId] {
        &self.event_content_ids
    }

    /// Returns the events emitted during the execution.
    pub fn events(&self) -> &[GatewayEvent] {
        &self.events
    }

    /// Returns the generated response id, if a response object was produced.
    pub fn response_id(&self) -> Option<&str> {
        self.response_id.as_deref()
    }

    /// Returns the response creation timestamp in milliseconds, if set.
    pub fn response_created_at_ms(&self) -> Option<u64> {
        self.response_created_at_ms
    }

    /// Returns the content id of the stored response, if it was recorded.
    pub fn response_content_id(&self) -> Option<&ContentId> {
        self.response_content_id.as_ref()
    }

    pub(crate) fn error(error: OpenAiRouteError) -> Self {
        Self {
            response: error.into_response(),
            request_content_id: None,
            run_content_id: None,
            event_content_ids: Vec::new(),
            events: Vec::new(),
            response_id: None,
            response_created_at_ms: None,
            response_content_id: None,
        }
    }
}

/// Extracts the `usage` field of a model response expression, defaulting to nil.
pub(crate) fn response_usage_expr(response: &Expr) -> Expr {
    response_field(response, "usage")
        .cloned()
        .unwrap_or(Expr::Nil)
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
