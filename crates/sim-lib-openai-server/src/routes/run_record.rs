#[cfg(test)]
use sim_kernel::ContentId;
use sim_kernel::{Expr, Symbol};

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

#[derive(Clone, Debug)]
pub(crate) struct RouteRunIdGenerators {
    request: GatewayIdGenerator,
    run: GatewayIdGenerator,
    event: GatewayIdGenerator,
}

impl RouteRunIdGenerators {
    pub(crate) fn deterministic(start: u64) -> Self {
        Self {
            request: GatewayIdGenerator::deterministic("gwreq", start),
            run: GatewayIdGenerator::deterministic("gwrun", start),
            event: GatewayIdGenerator::deterministic("gwevt", start),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RouteRunExecution {
    response: GatewayResponse,
    #[cfg(test)]
    request_content_id: Option<ContentId>,
    #[cfg(test)]
    run_content_id: Option<ContentId>,
    #[cfg(test)]
    event_content_ids: Vec<ContentId>,
    #[cfg(test)]
    events: Vec<GatewayEvent>,
    #[cfg(test)]
    response_content_id: Option<ContentId>,
}

impl RouteRunExecution {
    pub(crate) fn response(&self) -> &GatewayResponse {
        &self.response
    }

    #[cfg(test)]
    pub(crate) fn request_content_id(&self) -> Option<&ContentId> {
        self.request_content_id.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn run_content_id(&self) -> Option<&ContentId> {
        self.run_content_id.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn event_content_ids(&self) -> &[ContentId] {
        &self.event_content_ids
    }

    #[cfg(test)]
    pub(crate) fn events(&self) -> &[GatewayEvent] {
        &self.events
    }

    #[cfg(test)]
    pub(crate) fn response_content_id(&self) -> Option<&ContentId> {
        self.response_content_id.as_ref()
    }

    pub(crate) fn error(error: OpenAiRouteError) -> Self {
        Self {
            response: error.into_response(),
            #[cfg(test)]
            request_content_id: None,
            #[cfg(test)]
            run_content_id: None,
            #[cfg(test)]
            event_content_ids: Vec::new(),
            #[cfg(test)]
            events: Vec::new(),
            #[cfg(test)]
            response_content_id: None,
        }
    }
}

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
    let run = GatewayRun::new(
        run_id.clone(),
        request_content_id.clone(),
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    )
    .with_status(Symbol::new("completed"));
    let run_content_id = content_id_for_expr(&run.to_expr()).map_err(OpenAiRouteError::internal)?;
    if store_record {
        store
            .put_run(run_content_id.clone(), run)
            .map_err(OpenAiRouteError::internal)?;
    }

    let mut event_log = RouteEventLog::default();
    append_event(
        store,
        ids,
        clock,
        &run_id,
        0,
        "request-start",
        recorded_request.to_expr(),
        store_record,
        &mut event_log,
    )?;
    append_event(
        store,
        ids,
        clock,
        &run_id,
        1,
        "route-start",
        Expr::String(route.to_owned()),
        store_record,
        &mut event_log,
    )?;
    for (index, event) in route_events.into_iter().enumerate() {
        append_event(
            store,
            ids,
            clock,
            &run_id,
            index as u64 + 2,
            event.kind,
            event.payload,
            store_record,
            &mut event_log,
        )?;
    }
    append_event(
        store,
        ids,
        clock,
        &run_id,
        event_log.events.len() as u64,
        "final",
        final_payload(route, &response),
        store_record,
        &mut event_log,
    )?;

    #[cfg(test)]
    let response_content_id = if store_record {
        let id = content_id_for_expr(&response.to_expr()).map_err(OpenAiRouteError::internal)?;
        store
            .put_response(id.clone(), response.clone())
            .map_err(OpenAiRouteError::internal)?;
        Some(id)
    } else {
        None
    };
    #[cfg(not(test))]
    if store_record {
        let id = content_id_for_expr(&response.to_expr()).map_err(OpenAiRouteError::internal)?;
        store
            .put_response(id, response.clone())
            .map_err(OpenAiRouteError::internal)?;
    }

    Ok(RouteRunExecution {
        response,
        #[cfg(test)]
        request_content_id: Some(request_content_id),
        #[cfg(test)]
        run_content_id: Some(run_content_id),
        #[cfg(test)]
        event_content_ids: event_log.content_ids,
        #[cfg(test)]
        events: event_log.events,
        #[cfg(test)]
        response_content_id,
    })
}

#[derive(Default)]
struct RouteEventLog {
    #[cfg(test)]
    content_ids: Vec<ContentId>,
    events: Vec<GatewayEvent>,
}

#[allow(clippy::too_many_arguments)]
fn append_event<S, C>(
    store: &mut S,
    ids: &mut RouteRunIdGenerators,
    clock: &mut C,
    run_id: &str,
    sequence: u64,
    kind: &'static str,
    payload: Expr,
    store_event: bool,
    event_log: &mut RouteEventLog,
) -> RouteResult<()>
where
    S: GatewayStore,
    C: GatewayClock,
{
    let event = GatewayEvent::new(
        ids.event.next_id().map_err(OpenAiRouteError::internal)?,
        run_id,
        sequence,
        Symbol::new(kind),
        payload,
        clock.now_ms().map_err(OpenAiRouteError::internal)?,
    );
    let id = content_id_for_expr(&event.to_expr()).map_err(OpenAiRouteError::internal)?;
    if store_event {
        store
            .put_event(id.clone(), event.clone())
            .map_err(OpenAiRouteError::internal)?;
    }
    #[cfg(test)]
    event_log.content_ids.push(id);
    event_log.events.push(event);
    Ok(())
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
