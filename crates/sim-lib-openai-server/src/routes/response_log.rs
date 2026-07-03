use sim_kernel::{ContentId, Expr, Symbol};

use crate::{
    clock::GatewayClock,
    content_id::content_id_for_expr,
    objects::GatewayEvent,
    routes::{errors::OpenAiRouteError, responses::ResponseIdGenerators},
    storage::GatewayStore,
};

type RouteResult<T> = std::result::Result<T, OpenAiRouteError>;

pub(super) struct EventInput {
    sequence: u64,
    kind: Symbol,
    payload: Expr,
}

impl EventInput {
    pub(super) fn new(sequence: u64, kind: &'static str, payload: Expr) -> Self {
        Self::from_symbol(sequence, Symbol::new(kind), payload)
    }

    pub(super) fn from_symbol(sequence: u64, kind: Symbol, payload: Expr) -> Self {
        Self {
            sequence,
            kind,
            payload,
        }
    }
}

#[derive(Default)]
pub(super) struct EventLog {
    pub(super) content_ids: Vec<ContentId>,
    pub(super) events: Vec<GatewayEvent>,
}

pub(super) fn append_event<S, C>(
    store: &mut S,
    ids: &mut ResponseIdGenerators,
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

pub(super) fn response_usage_expr(response: &Expr) -> Expr {
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
