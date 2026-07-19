use sim_kernel::{DatumStore, Event, EventKind, Ref, Result, Symbol, stream_surface};
use sim_lib_server::{
    FrameEnvelope, FrameKind, ServerFrame, stream_chunk_frame_from_expr, stream_end_frame,
    stream_frame_from_expr, stream_frame_to_expr,
};
use sim_lib_stream_core::{
    ClockDomain, StreamCassette, StreamDirection, StreamEnvelope, StreamItem, StreamMedia,
    StreamMetadata, StreamRemoteLimits, StreamValue, TransportProfile,
    stream_remote_network_capability,
};

use crate::events::{
    diagnostic_stream_packet, error_message, metadata_from_expr, packet_and_ticks_from_remote_expr,
    packet_ref, remote_error_packet, stream_limit_diagnostic_kind,
};

/// Resource bounds applied while encoding a stream into server frames.
///
/// Mirrors `StreamRemoteLimits` from `sim-lib-stream-core` in fabric terms;
/// once a bound is crossed the encoder appends a limit diagnostic frame and
/// stops rather than emitting an unbounded run of chunks. The [`Default`] impl
/// inherits the stream-core remote defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFrameLimits {
    /// Maximum encoded payload size, in bytes, allowed for a single frame.
    pub max_frame_payload_bytes: usize,
    /// Maximum number of chunk frames emitted for one stream.
    pub max_stream_frames: usize,
    /// Maximum number of frames allowed in flight at once.
    pub max_inflight_frames: usize,
    /// Maximum stream duration, in milliseconds, before truncation.
    pub max_duration_ms: u64,
    /// Maximum stream rate, in hertz, for deriving the effective frame limit.
    pub max_rate_hz: u32,
}

impl Default for StreamFrameLimits {
    fn default() -> Self {
        let limits = StreamRemoteLimits::default();
        Self {
            max_frame_payload_bytes: limits.max_frame_payload_bytes,
            max_stream_frames: limits.max_stream_frames,
            max_inflight_frames: limits.max_inflight_frames,
            max_duration_ms: limits.max_duration_ms,
            max_rate_hz: limits.max_rate_hz,
        }
    }
}

impl StreamFrameLimits {
    /// Converts these fabric limits into a stream-core `StreamRemoteLimits`.
    ///
    /// The binary-payload bound is filled from the stream-core default.
    pub fn remote_limits(self) -> StreamRemoteLimits {
        StreamRemoteLimits {
            max_frame_payload_bytes: self.max_frame_payload_bytes,
            max_stream_frames: self.max_stream_frames,
            max_inflight_frames: self.max_inflight_frames,
            max_duration_ms: self.max_duration_ms,
            max_rate_hz: self.max_rate_hz,
            max_binary_payload_bytes: StreamRemoteLimits::default().max_binary_payload_bytes,
        }
    }
}

/// Encodes a stream into server frames with default envelope, profile, limits.
///
/// Convenience wrapper over [`stream_to_frames_with_envelope`] using
/// `FrameEnvelope::default()`.
pub fn stream_to_frames(
    cx: &mut sim_kernel::Cx,
    stream: &StreamValue,
    codec: Symbol,
) -> Result<Vec<ServerFrame>> {
    stream_to_frames_with_envelope(cx, stream, codec, FrameEnvelope::default())
}

/// Encodes a stream into server frames under a caller-supplied frame envelope.
///
/// Uses the remote-stream-fabric transport profile and default
/// [`StreamFrameLimits`]; see [`stream_to_frames_with_limits`] for the full
/// surface.
pub fn stream_to_frames_with_envelope(
    cx: &mut sim_kernel::Cx,
    stream: &StreamValue,
    codec: Symbol,
    envelope: FrameEnvelope,
) -> Result<Vec<ServerFrame>> {
    stream_to_frames_with_limits(
        cx,
        stream,
        codec,
        envelope,
        TransportProfile::remote_stream_fabric(),
        StreamFrameLimits::default(),
    )
}

/// Encodes a stream into server frames under a caller-chosen transport profile.
///
/// Uses default [`StreamFrameLimits`]; see [`stream_to_frames_with_limits`].
pub fn stream_to_frames_with_profile(
    cx: &mut sim_kernel::Cx,
    stream: &StreamValue,
    codec: Symbol,
    envelope: FrameEnvelope,
    profile: TransportProfile,
) -> Result<Vec<ServerFrame>> {
    stream_to_frames_with_limits(
        cx,
        stream,
        codec,
        envelope,
        profile,
        StreamFrameLimits::default(),
    )
}

/// Encodes a stream into server frames, enforcing every fabric resource bound.
///
/// Emits a `StreamStart` frame carrying metadata, one chunk frame per packet,
/// and a terminating `StreamEnd` frame. Requires the remote-network capability.
/// When any of `limits` (frame size, stream size, inflight count, duration or
/// rate) is exceeded, a limit diagnostic frame is appended and encoding stops
/// rather than producing an unbounded stream.
pub fn stream_to_frames_with_limits(
    cx: &mut sim_kernel::Cx,
    stream: &StreamValue,
    codec: Symbol,
    envelope: FrameEnvelope,
    profile: TransportProfile,
    limits: StreamFrameLimits,
) -> Result<Vec<ServerFrame>> {
    cx.require(&stream_remote_network_capability())?;
    let remote_limits = limits.remote_limits();
    remote_limits.validate()?;
    let effective_frame_limit = remote_limits.effective_frame_limit();
    let mut frames = Vec::new();
    frames.push(stream_frame_from_expr(
        cx,
        codec.clone(),
        FrameKind::StreamStart,
        &stream.metadata().table_expr(),
        envelope.clone(),
    )?);
    let mut sequence = 0_u64;
    while let Some(item) = stream.next_packet()? {
        if sequence as usize >= effective_frame_limit {
            frames.push(limit_diagnostic_frame(
                cx,
                codec.clone(),
                stream.metadata(),
                sequence,
                stream_size_limit_message(&limits, effective_frame_limit),
                envelope.clone(),
            )?);
            break;
        }
        if frames.len() >= limits.max_inflight_frames {
            frames.push(limit_diagnostic_frame(
                cx,
                codec.clone(),
                stream.metadata(),
                sequence,
                format!(
                    "stream/fabric inflight-frame limit exceeded at {} frames",
                    limits.max_inflight_frames
                ),
                envelope.clone(),
            )?);
            break;
        }
        let frame = envelope_chunk_frame(
            cx,
            codec.clone(),
            stream.metadata(),
            sequence,
            &item,
            profile.clone(),
            envelope.clone(),
        )?;
        if frame.payload.len() > limits.max_frame_payload_bytes {
            frames.push(limit_diagnostic_frame(
                cx,
                codec.clone(),
                stream.metadata(),
                sequence,
                format!(
                    "stream/fabric frame-size limit exceeded: {} bytes > {} bytes",
                    frame.payload.len(),
                    limits.max_frame_payload_bytes
                ),
                envelope.clone(),
            )?);
            break;
        }
        frames.push(frame);
        sequence = sequence.saturating_add(1);
    }
    frames.push(stream_end_frame(codec, envelope));
    Ok(frames)
}

fn stream_size_limit_message(limits: &StreamFrameLimits, effective_frame_limit: usize) -> String {
    if effective_frame_limit < limits.max_stream_frames {
        format!("stream/fabric duration-rate limit exceeded after {effective_frame_limit} chunks")
    } else {
        format!(
            "stream/fabric stream-size limit exceeded after {} chunks",
            limits.max_stream_frames
        )
    }
}

/// Compatibility helper. New code should call
/// `sim_lib_server::stream_chunk_frame_from_expr` directly.
pub fn expr_to_stream_chunk_frame(
    cx: &mut sim_kernel::Cx,
    codec: Symbol,
    expr: sim_kernel::Expr,
    envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    stream_chunk_frame_from_expr(cx, codec, &expr, envelope)
}

/// Compatibility helper. New code should call
/// `sim_lib_server::stream_frame_to_expr` directly.
pub fn stream_chunk_frame_to_expr(
    cx: &mut sim_kernel::Cx,
    frame: &ServerFrame,
) -> Result<sim_kernel::Expr> {
    if frame.kind != FrameKind::StreamChunk {
        return Err(sim_kernel::Error::Eval(format!(
            "remote stream adapter expected stream chunk frame, got {}",
            frame.kind.as_symbol()
        )));
    }
    let Some(expr) = stream_frame_to_expr(cx, frame)? else {
        return Err(sim_kernel::Error::Eval(
            "stream chunk frame did not decode to a payload".to_owned(),
        ));
    };
    Ok(expr)
}

/// Decodes a buffer of server frames back into a lazy [`StreamValue`].
///
/// Reconstructs metadata from the `StreamStart` frame, turns the remaining
/// frames into events, and folds them into a pull-based stream.
pub fn stream_frames_to_stream(
    cx: &mut sim_kernel::Cx,
    frames: &[ServerFrame],
) -> Result<StreamValue> {
    let run = Ref::Symbol(Symbol::qualified("stream/fabric", "remote-run"));
    let (metadata, events) = stream_frames_to_events(cx, frames, run)?;
    crate::event_buffer_to_stream(cx, metadata, events)
}

/// Decodes server frames into a replayable [`StreamCassette`].
///
/// Recovers the stream via [`stream_frames_to_stream`] and records it under the
/// remote-stream-fabric transport profile.
pub fn stream_frames_to_cassette(
    cx: &mut sim_kernel::Cx,
    frames: &[ServerFrame],
) -> Result<StreamCassette> {
    let stream = stream_frames_to_stream(cx, frames)?;
    StreamCassette::from_stream_value(&stream, TransportProfile::remote_stream_fabric())
}

/// Re-encodes a recorded [`StreamCassette`] back into server frames.
///
/// Replays the cassette and re-emits it under the remote-stream-fabric profile.
pub fn cassette_to_stream_frames(
    cx: &mut sim_kernel::Cx,
    cassette: &StreamCassette,
    codec: Symbol,
    envelope: FrameEnvelope,
) -> Result<Vec<ServerFrame>> {
    let stream = cassette.replay_stream_value()?;
    stream_to_frames_with_profile(
        cx,
        &stream,
        codec,
        envelope,
        TransportProfile::remote_stream_fabric(),
    )
}

/// Decodes server frames into stream metadata plus a buffer of run events.
///
/// The `StreamStart` frame supplies metadata; chunk, end, and error frames
/// become events attributed to `run`, with decoding stopping at the first
/// `Done`. Returns an error if no `StreamStart` metadata frame is present.
pub fn stream_frames_to_events(
    cx: &mut sim_kernel::Cx,
    frames: &[ServerFrame],
    run: Ref,
) -> Result<(StreamMetadata, Vec<Event>)> {
    let mut metadata = None;
    let mut events = Vec::new();
    let mut seq = 0u64;
    for frame in frames {
        match frame.kind {
            FrameKind::StreamStart => {
                let expr = frame.decode_expr(cx, sim_kernel::ReadPolicy::default())?;
                metadata = Some(metadata_from_expr(&expr)?);
            }
            FrameKind::StreamChunk | FrameKind::StreamEnd | FrameKind::Error => {
                if let Some(event) = remote_frame_to_event(cx, run.clone(), seq, frame)? {
                    let done = matches!(event.kind, EventKind::Done);
                    events.push(event);
                    seq = seq.saturating_add(1);
                    if done {
                        break;
                    }
                }
            }
            _ => {
                return Err(sim_kernel::Error::Eval(format!(
                    "remote stream adapter cannot consume frame kind {}",
                    frame.kind.as_symbol()
                )));
            }
        }
    }
    let metadata = metadata.ok_or_else(|| {
        sim_kernel::Error::Eval("remote stream frames missing StreamStart metadata".to_owned())
    })?;
    Ok((metadata, events))
}

/// Converts a single remote server frame into a run event, if it carries one.
///
/// Chunk frames yield a remote stream-frame event, end frames yield `Done`, and
/// error frames yield a remote-error diagnostic event. `StreamStart` frames
/// carry no event and return `None`.
pub fn remote_frame_to_event(
    cx: &mut sim_kernel::Cx,
    run: Ref,
    seq: u64,
    frame: &ServerFrame,
) -> Result<Option<Event>> {
    match frame.kind {
        FrameKind::StreamChunk => {
            let Some(expr) = stream_frame_to_expr(cx, frame)? else {
                return Err(sim_kernel::Error::Eval(
                    "stream chunk frame did not decode to a payload".to_owned(),
                ));
            };
            let (packet, ticks) = packet_and_ticks_from_remote_expr(expr);
            Ok(Some(stream_surface::remote_stream_frame_event(
                run,
                seq,
                ticks,
                packet_ref(cx, &packet)?,
            )?))
        }
        FrameKind::StreamEnd => Ok(Some(Event::done(run, seq)?)),
        FrameKind::Error => {
            let packet = remote_error_packet(remote_error_message(cx, frame));
            Ok(Some(stream_surface::remote_stream_frame_event(
                run,
                seq,
                Vec::new(),
                packet_ref(cx, &packet)?,
            )?))
        }
        FrameKind::StreamStart => Ok(None),
        _ => Err(sim_kernel::Error::Eval(format!(
            "remote stream adapter cannot convert frame kind {}",
            frame.kind.as_symbol()
        ))),
    }
}

fn remote_error_message(cx: &mut sim_kernel::Cx, frame: &ServerFrame) -> String {
    match frame.decode_expr(cx, sim_kernel::ReadPolicy::default()) {
        Ok(expr) => match sim_kernel::Datum::try_from(expr) {
            Ok(datum) => match cx.datum_store_mut().intern(datum) {
                Ok(id) => error_message(cx, &Ref::Content(id)),
                Err(err) => err.to_string(),
            },
            Err(err) => err.to_string(),
        },
        Err(err) => err.to_string(),
    }
}

fn envelope_chunk_frame(
    cx: &mut sim_kernel::Cx,
    codec: Symbol,
    metadata: &StreamMetadata,
    sequence: u64,
    item: &StreamItem,
    profile: TransportProfile,
    frame_envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    let envelope = StreamEnvelope::from_item_with_profile(metadata, sequence, item, profile)?;
    stream_chunk_frame_from_expr(cx, codec, &envelope.to_expr(), frame_envelope)
}

fn limit_diagnostic_frame(
    cx: &mut sim_kernel::Cx,
    codec: Symbol,
    metadata: &StreamMetadata,
    sequence: u64,
    message: String,
    frame_envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    let kind = stream_limit_diagnostic_kind();
    let packet = diagnostic_stream_packet(kind.clone(), message);
    let envelope = StreamEnvelope::new(
        metadata.id().clone(),
        Symbol::qualified(
            "stream/packet-id",
            format!("{}#diagnostic-{sequence}", metadata.id().as_qualified_str()),
        ),
        StreamMedia::Diagnostic,
        StreamDirection::Source,
        sequence,
        Vec::new(),
        ClockDomain::ServerFrame,
        TransportProfile::remote_stream_fabric(),
        vec![kind],
        packet,
    )?;
    stream_chunk_frame_from_expr(cx, codec, &envelope.to_expr(), frame_envelope)
}
