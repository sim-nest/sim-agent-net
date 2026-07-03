use sim_kernel::{
    Datum, Diagnostic, Error, Event, EventKind, Expr, Ref, Result, Severity, Symbol, Tick,
    value_from_ref,
};
use sim_lib_stream_core::{
    StreamCapability, StreamDiagnostic, StreamEnvelope, StreamItem, StreamMetadata, StreamPacket,
    StreamValue,
};

/// Returns the diagnostic kind marking a remote-side error surfaced locally.
pub fn remote_error_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/fabric", "RemoteError")
}

/// Returns the diagnostic kind marking a transport profile refused over frames.
pub fn refused_profile_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/fabric", "RefusedProfile")
}

/// Returns the diagnostic kind marking a stream truncated by a fabric limit.
pub fn stream_limit_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/fabric", "LimitExceeded")
}

/// Builds a chunk event whose payload is the interned data packet `(kind expr)`.
///
/// Used to inject an arbitrary `Expr` into a run's event stream at sequence
/// `seq` under the realize event surface.
pub fn expr_chunk_event(
    cx: &mut sim_kernel::Cx,
    run: Ref,
    seq: u64,
    kind: Symbol,
    expr: Expr,
) -> Result<Event> {
    let packet = StreamPacket::data(kind, expr);
    Event::new(
        run,
        seq,
        Vec::new(),
        EventKind::Chunk {
            payload: packet.intern_ref(cx)?,
        },
    )
}

/// Folds a buffer of run events into a pull-based [`StreamValue`].
///
/// Chunk events become stream items, diagnostic events become diagnostic
/// packets, and a failure or `Done` event ends the stream. Non-stream event
/// kinds (started, claim, trace, effect, capture, card, final) are ignored.
pub fn event_buffer_to_stream(
    cx: &mut sim_kernel::Cx,
    metadata: StreamMetadata,
    events: impl IntoIterator<Item = Event>,
) -> Result<StreamValue> {
    let mut items = Vec::new();
    for event in events {
        match event.kind {
            EventKind::Chunk { payload } => {
                let packet = packet_from_ref(cx, &payload)?;
                items.push(StreamItem::with_ticks(packet, event.ticks)?);
            }
            EventKind::Diagnostic(diagnostic) => {
                items.push(StreamItem::new(StreamPacket::Diagnostic(
                    diagnostic_packet(diagnostic),
                )));
            }
            EventKind::Failed(error) => {
                items.push(StreamItem::new(remote_error_packet(error_message(
                    cx, &error,
                ))));
                break;
            }
            EventKind::Done => break,
            EventKind::Started { .. }
            | EventKind::Claim { .. }
            | EventKind::Trace(_)
            | EventKind::EffectRequested { .. }
            | EventKind::EffectResolved { .. }
            | EventKind::Capture { .. }
            | EventKind::Card { .. }
            | EventKind::Final(_) => {}
        }
    }
    Ok(StreamValue::pull(metadata, items))
}

pub(crate) fn packet_from_ref(cx: &mut sim_kernel::Cx, reference: &Ref) -> Result<StreamPacket> {
    let value = match value_from_ref(cx, reference) {
        Ok(value) => value,
        Err(error) => {
            return Ok(chunk_payload_error_packet(format!(
                "stream chunk payload ref could not be loaded: {error}"
            )));
        }
    };
    let expr = match value.object().as_expr(cx) {
        Ok(expr) => expr,
        Err(error) => {
            return Ok(chunk_payload_error_packet(format!(
                "stream chunk payload could not be represented as Expr: {error}"
            )));
        }
    };
    Ok(packet_from_expr(expr))
}

pub(crate) fn packet_and_ticks_from_remote_expr(expr: Expr) -> (StreamPacket, Vec<Tick>) {
    match StreamEnvelope::try_from(expr.clone()) {
        Ok(envelope) => match remote_profile_refusal(&envelope) {
            Some(message) => (
                diagnostic_stream_packet(refused_profile_diagnostic_kind(), message),
                Vec::new(),
            ),
            None => (envelope.packet().clone(), envelope.ticks().to_vec()),
        },
        Err(_) => (packet_from_expr(expr), Vec::new()),
    }
}

pub(crate) fn packet_from_expr(expr: Expr) -> StreamPacket {
    match StreamPacket::try_from(expr.clone()) {
        Ok(packet) => packet,
        Err(packet_error) => match Datum::try_from(expr.clone()) {
            Ok(_) => StreamPacket::data(Symbol::qualified("stream/data", "expr"), expr),
            Err(datum_error) => chunk_payload_error_packet(format!(
                "stream chunk payload is neither a stream packet nor datum Expr: packet parse failed: {packet_error}; datum conversion failed: {datum_error}"
            )),
        },
    }
}

pub(crate) fn diagnostic_stream_packet(kind: Symbol, message: impl Into<String>) -> StreamPacket {
    StreamPacket::Diagnostic(StreamDiagnostic::new(kind, message.into()))
}

pub(crate) fn remote_error_packet(message: impl Into<String>) -> StreamPacket {
    diagnostic_stream_packet(remote_error_diagnostic_kind(), message)
}

fn chunk_payload_error_packet(message: impl Into<String>) -> StreamPacket {
    diagnostic_stream_packet(
        Symbol::qualified("stream/fabric", "ChunkPayloadError"),
        message.into(),
    )
}

pub(crate) fn error_message(cx: &mut sim_kernel::Cx, reference: &Ref) -> String {
    match value_from_ref(cx, reference) {
        Ok(value) => value
            .object()
            .display(cx)
            .unwrap_or_else(|err| err.to_string()),
        Err(err) => err.to_string(),
    }
}

fn diagnostic_packet(diagnostic: Diagnostic) -> StreamDiagnostic {
    let kind = diagnostic
        .code
        .unwrap_or_else(|| Symbol::qualified("stream/fabric", "Diagnostic"));
    let prefix = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Note => "note",
    };
    StreamDiagnostic::new(kind, format!("{prefix}: {}", diagnostic.message))
}

pub(crate) fn packet_ref(cx: &mut sim_kernel::Cx, packet: &StreamPacket) -> Result<Ref> {
    packet.intern_ref(cx)
}

pub(crate) fn metadata_from_expr(expr: &Expr) -> Result<StreamMetadata> {
    StreamMetadata::from_table_expr(expr).map_err(|err| {
        Error::Eval(format!(
            "remote stream start frame did not contain stream metadata: {err}"
        ))
    })
}

fn remote_profile_refusal(envelope: &StreamEnvelope) -> Option<String> {
    let profile = envelope.profile();
    if profile.name().as_qualified_str() == "stream/profile/realtime-local-audio"
        || profile.has_capability(StreamCapability::Realtime)
            && !profile.has_capability(StreamCapability::Preview)
    {
        return Some(format!(
            "stream profile {} is refused over remote stream fabric frames; convert to stream/profile/lan-buffered-audio-preview chunks",
            profile.name().as_qualified_str()
        ));
    }
    None
}
