use sim_kernel::{
    Datum, DatumStore, Error, Event, Expr, ObserveMode, RealizeRequest, Ref, Result, Symbol, Term,
};
use sim_lib_server::Site;
use sim_lib_stream_core::{PlacedFragment, StreamEnvelope, StreamValue};

/// Builds a realize request that observes a term as an event stream.
///
/// The request observes in `ObserveMode::Events` and carries an optional
/// `buffer_limit`, framing the term for the location-transparent eval surface
/// to drive as a stream.
pub fn stream_realize_request(term: Term, buffer_limit: Option<usize>) -> RealizeRequest {
    let mut request = RealizeRequest::new(term).observing(ObserveMode::Events);
    request.buffer_limit = buffer_limit;
    request
}

/// Drains a stream into the events of one realized run.
///
/// Emits a `started` event, one chunk event per packet up to the request's
/// answer limit, and a `done` event once the stream reports completion.
/// Requires the request to observe in `ObserveMode::Events`.
pub fn realize_stream_events(
    cx: &mut sim_kernel::Cx,
    request: &RealizeRequest,
    stream: &StreamValue,
) -> Result<Vec<Event>> {
    if request.observe != ObserveMode::Events {
        return Err(Error::Eval(
            "stream realization requires ObserveMode::Events".to_owned(),
        ));
    }
    let run = Ref::Handle(cx.fresh_handle());
    let request_ref = realize_request_ref(cx, request)?;
    let mut events = vec![Event::started(run.clone(), 0, request_ref)?];
    let mut seq = 1u64;
    let mut emitted = 0usize;
    let limit = request.answer_limit.unwrap_or(usize::MAX);
    while emitted < limit {
        let Some(item) = stream.next_packet()? else {
            break;
        };
        events.push(item.chunk_event(cx, run.clone(), seq)?);
        seq = seq.saturating_add(1);
        emitted += 1;
    }
    if stream.is_done()? {
        events.push(Event::done(run, seq)?);
    }
    Ok(events)
}

/// Realizes a placed fragment on `site` and returns its output envelopes.
///
/// Delegates to `sim_lib_server::realize_stream_events`, keeping placed-stream
/// realization on the realize surface rather than a transport-specific API.
pub fn realize_placed_stream_events(
    cx: &mut sim_kernel::Cx,
    fragment: PlacedFragment,
    site: &dyn Site,
) -> Result<Vec<StreamEnvelope>> {
    sim_lib_server::realize_stream_events(cx, fragment, site)
}

fn realize_request_ref(cx: &mut sim_kernel::Cx, request: &RealizeRequest) -> Result<Ref> {
    let id = cx.datum_store_mut().intern(Datum::Node {
        tag: Symbol::qualified("stream/fabric", "RealizeRequest"),
        fields: vec![
            (
                Symbol::new("term"),
                Datum::try_from(Expr::from(request.term.clone()))?,
            ),
            (
                Symbol::new("observe"),
                Datum::String(observe_name(request.observe).to_owned()),
            ),
            (
                Symbol::new("buffer-limit"),
                optional_usize_datum(request.buffer_limit),
            ),
            (
                Symbol::new("answer-limit"),
                optional_usize_datum(request.answer_limit),
            ),
        ],
    })?;
    Ok(Ref::Content(id))
}

fn optional_usize_datum(value: Option<usize>) -> Datum {
    value.map_or(Datum::Nil, |value| Datum::String(value.to_string()))
}

fn observe_name(observe: ObserveMode) -> &'static str {
    match observe {
        ObserveMode::FinalOnly => "final-only",
        ObserveMode::Events => "events",
        ObserveMode::Ledger => "ledger",
    }
}
