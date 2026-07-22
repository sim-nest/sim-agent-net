use sim_kernel::{Expr, Symbol};

use crate::{
    clock::GatewayClock,
    content_id::content_id_for_expr,
    objects::{GatewayRequest, GatewayResponse},
    storage::GatewayStore,
};

use super::{
    errors::OpenAiRouteError,
    execution_record::{EventInput, EventLog, RunPrologue, append_event, begin_run},
};

/// The run_record engine (audio/images/vector_stores) shares the gateway
/// execution-record substrate; its id generators and execution outcome are the
/// shared types under route-local names.
pub(crate) use super::execution_record::{
    GatewayRunExecution as RouteRunExecution, GatewayRunIdGenerators as RouteRunIdGenerators,
};

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

/// One route-specific event to append between the shared `route-start` and
/// `final` events: its kind and payload (the sequence is assigned by the
/// substrate).
#[derive(Clone, Debug)]
pub(crate) struct RouteEventInput {
    kind: &'static str,
    payload: Expr,
}

impl RouteEventInput {
    pub(crate) fn new(kind: &'static str, payload: Expr) -> Self {
        Self { kind, payload }
    }
}

/// A fully-formed run to record: the wire response, the route path, the
/// route-specific events, and whether to persist the run.
pub(crate) struct RouteRunRecord {
    response: GatewayResponse,
    route: &'static str,
    route_events: Vec<RouteEventInput>,
    store_record: bool,
}

impl RouteRunRecord {
    pub(crate) fn new(
        response: GatewayResponse,
        route: &'static str,
        route_events: Vec<RouteEventInput>,
        store_record: bool,
    ) -> Self {
        Self {
            response,
            route,
            route_events,
            store_record,
        }
    }
}

/// Records a run_record execution: opens the run (marked `completed`), appends
/// the shared `request-start`/`route-start` events, the route's own events, and
/// a `final` event, then stores the response BARE via `put_response` (the
/// object-vs-bare split that stays route-local).
pub(crate) fn record_route_execution<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    request: &GatewayRequest,
    record: RouteRunRecord,
) -> RouteResult<RouteRunExecution>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let RouteRunRecord {
        response,
        route,
        route_events,
        store_record,
    } = record;

    let RunPrologue {
        recorded_request,
        request_content_id,
        run_id,
        run_content_id,
    } = begin_run(
        store,
        ids,
        clock,
        request,
        Some(Symbol::new("completed")),
        store_record,
    )?;

    let mut event_log = EventLog::default();
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(0, "request-start", recorded_request.to_expr()),
        store_record,
        &mut event_log,
    )?;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(1, "route-start", Expr::String(route.to_owned())),
        store_record,
        &mut event_log,
    )?;
    for (index, event) in route_events.into_iter().enumerate() {
        append_event(
            store,
            ids,
            clock,
            &run_id,
            EventInput::new(index as u64 + 2, event.kind, event.payload),
            store_record,
            &mut event_log,
        )?;
    }
    append_event(
        store,
        ids,
        clock,
        &run_id,
        EventInput::new(
            event_log.events.len() as u64,
            "final",
            final_payload(route, &response),
        ),
        store_record,
        &mut event_log,
    )?;

    let response_content_id = if store_record {
        let id = content_id_for_expr(&response.to_expr()).map_err(OpenAiRouteError::internal)?;
        store
            .put_response(id.clone(), response.clone())
            .map_err(OpenAiRouteError::internal)?;
        Some(id)
    } else {
        None
    };

    Ok(RouteRunExecution {
        response,
        request_content_id: Some(request_content_id),
        run_content_id: Some(run_content_id),
        event_content_ids: event_log.content_ids,
        events: event_log.events,
        response_id: None,
        response_created_at_ms: None,
        response_content_id,
    })
}

fn final_payload(route: &'static str, response: &GatewayResponse) -> Expr {
    Expr::Map(vec![
        field("route", Expr::String(route.to_owned())),
        field("status", Expr::String(response.status().to_string())),
        field(
            "body-bytes",
            Expr::String(response.body().len().to_string()),
        ),
    ])
}

pub(crate) use sim_value::build::entry as field;
