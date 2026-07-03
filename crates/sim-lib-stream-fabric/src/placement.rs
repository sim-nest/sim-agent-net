use std::time::Duration;

use sim_kernel::{Cx, Error, Expr, Result, Symbol};
use sim_lib_server::{
    FrameEnvelope, FrameKind, ServerFrame, Site, stream_chunk_frame_from_expr, stream_end_frame,
    stream_frame_from_expr,
};
use sim_lib_stream_core::{
    BufferPolicy, ClockDomain, LatencyClass, PlacedFragment, StreamCapability, StreamDirection,
    StreamEnvelope, StreamMedia, StreamMetadata, StreamPacket, TransportProfile,
    stream_remote_network_capability, stream_remote_preview_capability,
};

use crate::events::diagnostic_stream_packet;
use crate::placement_report::{
    placement_uses_host_device, request_report_expr, result_report_expr,
};
use crate::placement_security::{
    PlacementResourceLimits, placement_host_device_capability, placement_remote_render_capability,
    placement_run_node_on_server_capability, redact_placement_symbol,
};

/// Server placement request carried through stream-fabric server frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerPlacementRequest {
    placement_report: Expr,
    output_profile: TransportProfile,
    realtime_pinned: bool,
    resource_limits: PlacementResourceLimits,
}

impl ServerPlacementRequest {
    /// Creates a request for `placement_report` declaring `output_profile`.
    ///
    /// Realtime pinning is off and resource limits start at their defaults.
    pub fn new(placement_report: Expr, output_profile: TransportProfile) -> Self {
        Self {
            placement_report,
            output_profile,
            realtime_pinned: false,
            resource_limits: PlacementResourceLimits::default(),
        }
    }

    /// Creates a request whose output uses the server buffered-preview profile.
    pub fn buffered_preview(placement_report: Expr) -> Self {
        Self::new(placement_report, server_buffered_preview_profile())
    }

    /// Creates a request whose output uses the server render-return profile.
    pub fn render_return(placement_report: Expr) -> Self {
        Self::new(placement_report, server_render_return_profile())
    }

    /// Returns this request with realtime pinning set to `realtime_pinned`.
    ///
    /// A realtime-pinned node cannot be hosted on the server site and is
    /// refused during placement.
    pub fn with_realtime_pin(mut self, realtime_pinned: bool) -> Self {
        self.realtime_pinned = realtime_pinned;
        self
    }

    /// Returns this request with its resource limits replaced.
    pub fn with_resource_limits(mut self, resource_limits: PlacementResourceLimits) -> Self {
        self.resource_limits = resource_limits;
        self
    }

    /// Returns the placement report `Expr` describing the work to place.
    pub fn placement_report(&self) -> &Expr {
        &self.placement_report
    }

    /// Returns the declared output transport profile.
    pub fn output_profile(&self) -> &TransportProfile {
        &self.output_profile
    }

    /// Returns whether the placed node is pinned to realtime execution.
    pub fn realtime_pinned(&self) -> bool {
        self.realtime_pinned
    }

    /// Returns the resource limits governing this placement.
    pub fn resource_limits(&self) -> PlacementResourceLimits {
        self.resource_limits
    }
}

/// Returns the site symbol naming the server placement target.
pub fn server_site_symbol() -> Symbol {
    Symbol::qualified("stream/site", "server")
}

/// Returns the operation symbol for a server placement request data frame.
pub fn server_placement_request_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "server-placement-request")
}

/// Returns the operation symbol for a server placement result data frame.
pub fn server_placement_result_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "server-placement-result")
}

/// Returns the operation symbol for a server placement refusal data frame.
pub fn server_placement_refusal_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "server-placement-refusal")
}

/// Returns the request, result, and refusal placement operation symbols.
pub fn server_placement_frame_operation_symbols() -> [Symbol; 3] {
    [
        server_placement_request_symbol(),
        server_placement_result_symbol(),
        server_placement_refusal_symbol(),
    ]
}

/// Returns the diagnostic kind emitted when server placement is refused.
pub fn server_placement_refusal_diagnostic_kind() -> Symbol {
    Symbol::qualified("stream/fabric", "ServerPlacementRefused")
}

/// Returns the server buffered-preview output profile.
///
/// A remote, bounded, lossy preview profile in the buffered-preview latency
/// class, suitable for server-hosted placement that streams a preview back.
pub fn server_buffered_preview_profile() -> TransportProfile {
    TransportProfile::new(
        Symbol::qualified("stream/profile", "server-buffered-preview"),
        LatencyClass::BufferedPreview,
        vec![
            StreamCapability::Remote,
            StreamCapability::Bounded,
            StreamCapability::Preview,
            StreamCapability::Lossy,
        ],
    )
    .expect("server-buffered-preview stream profile is valid")
}

/// Returns the server render-return output profile.
///
/// A remote, bounded, deterministic, replayable, resumable profile in the
/// offline-render latency class, suitable for returning a fully rendered result.
pub fn server_render_return_profile() -> TransportProfile {
    TransportProfile::new(
        Symbol::qualified("stream/profile", "server-render-return"),
        LatencyClass::OfflineRender,
        vec![
            StreamCapability::Remote,
            StreamCapability::Bounded,
            StreamCapability::Deterministic,
            StreamCapability::Replayable,
            StreamCapability::Resumable,
        ],
    )
    .expect("server-render-return stream profile is valid")
}

/// Places a fragment on the server site and encodes the exchange as frames.
///
/// Requires the run-node-on-server and remote-network capabilities (plus
/// host-device access when the fragment touches a host device), validates the
/// request's resource limits, then emits a `StreamStart` frame, a redacted
/// request-report frame, a result-report frame for the realized node, one chunk
/// frame per output envelope, and a closing `StreamEnd` frame. Realtime-pinned
/// requests and any resource-limit breach short-circuit to a refusal frame
/// followed by `StreamEnd` instead of executing the node.
pub fn server_placement_stream_frames(
    cx: &mut Cx,
    site: &dyn Site,
    fragment: PlacedFragment,
    request: ServerPlacementRequest,
    codec: Symbol,
    frame_envelope: FrameEnvelope,
) -> Result<Vec<ServerFrame>> {
    cx.require(&placement_run_node_on_server_capability())?;
    cx.require(&stream_remote_network_capability())?;
    require_host_device_access(cx, &fragment, &request)?;
    let limits = request.resource_limits();
    limits.validate()?;
    let metadata = placement_metadata(&fragment, limits);
    let mut frames = vec![stream_frame_from_expr(
        cx,
        codec.clone(),
        FrameKind::StreamStart,
        &metadata.table_expr(),
        frame_envelope.clone(),
    )?];
    let mut accounted_payload_bytes = frames
        .first()
        .map(|frame| frame.payload.len())
        .unwrap_or_default();
    let mut data_frame_count = 0_usize;
    let mut sequence = 0_u64;
    let request_frame = data_frame(
        cx,
        codec.clone(),
        &metadata,
        sequence,
        server_placement_request_symbol(),
        request_report_expr(&fragment, &request),
        frame_envelope.clone(),
    )?;
    if let Some(message) = limit_cutoff_message(
        &request_frame,
        limits,
        &mut accounted_payload_bytes,
        data_frame_count,
        frames.len(),
    ) {
        return finish_with_refusal(
            cx,
            codec,
            &metadata,
            sequence,
            message,
            frame_envelope,
            frames,
        );
    }
    frames.push(request_frame);
    data_frame_count = data_frame_count.saturating_add(1);
    sequence = sequence.saturating_add(1);

    if request.realtime_pinned {
        return finish_with_refusal(
            cx,
            codec,
            &metadata,
            sequence,
            format!(
                "server placement refused fragment {}: stream/site/server cannot host a realtime-pinned node",
                redacted_fragment_label(&fragment)
            ),
            frame_envelope,
            frames,
        );
    }

    require_server_output_profile(cx, request.output_profile())?;
    site.accept_input_edges(fragment.input_edges())?;
    let reply = site.realize_fragment_node_with_timeout(
        cx,
        &fragment,
        Some(Duration::from_millis(limits.max_cpu_time_ms)),
    )?;
    let payload = reply.value.object().as_expr(cx)?;
    let result_frame = data_frame(
        cx,
        codec.clone(),
        &metadata,
        sequence,
        server_placement_result_symbol(),
        result_report_expr(&fragment, payload, request.output_profile()),
        frame_envelope.clone(),
    )?;
    if let Some(message) = limit_cutoff_message(
        &result_frame,
        limits,
        &mut accounted_payload_bytes,
        data_frame_count,
        frames.len(),
    ) {
        return finish_with_refusal(
            cx,
            codec,
            &metadata,
            sequence,
            message,
            frame_envelope,
            frames,
        );
    }
    frames.push(result_frame);
    data_frame_count = data_frame_count.saturating_add(1);
    sequence = sequence.saturating_add(1);

    for envelope in fragment.output_envelopes() {
        let output = envelope_with_profile(&envelope, request.output_profile().clone())?;
        let frame = stream_chunk_frame_from_expr(
            cx,
            codec.clone(),
            &output.to_expr(),
            frame_envelope.clone(),
        )?;
        if let Some(message) = limit_cutoff_message(
            &frame,
            limits,
            &mut accounted_payload_bytes,
            data_frame_count,
            frames.len(),
        ) {
            return finish_with_refusal(
                cx,
                codec,
                &metadata,
                sequence,
                message,
                frame_envelope,
                frames,
            );
        }
        frames.push(frame);
        data_frame_count = data_frame_count.saturating_add(1);
        sequence = sequence.saturating_add(1);
    }

    frames.push(stream_end_frame(codec, frame_envelope));
    Ok(frames)
}

fn placement_metadata(
    fragment: &PlacedFragment,
    limits: PlacementResourceLimits,
) -> StreamMetadata {
    StreamMetadata::new(
        redact_placement_symbol(&Symbol::qualified(
            "stream/server-placement",
            fragment.id().as_qualified_str(),
        )),
        StreamMedia::Data,
        StreamDirection::Source,
        ClockDomain::ServerFrame.symbol(),
        BufferPolicy::bounded(limits.max_inflight_work)
            .expect("validated placement inflight-work limit is nonzero"),
    )
}

fn require_server_output_profile(cx: &mut Cx, profile: &TransportProfile) -> Result<()> {
    if profile.has_capability(StreamCapability::Realtime)
        || profile.latency_class() == LatencyClass::SampleExact
    {
        return Err(Error::Eval(format!(
            "server placement refuses realtime profile {}",
            profile.name().as_qualified_str()
        )));
    }
    match profile.latency_class() {
        LatencyClass::BufferedPreview => cx.require(&stream_remote_preview_capability()),
        LatencyClass::OfflineRender => cx.require(&placement_remote_render_capability()),
        LatencyClass::Interactive => Ok(()),
        latency => Err(Error::Eval(format!(
            "server placement does not support {} latency",
            latency.wire_label()
        ))),
    }
}

fn data_frame(
    cx: &mut Cx,
    codec: Symbol,
    metadata: &StreamMetadata,
    sequence: u64,
    kind: Symbol,
    payload: Expr,
    frame_envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    let packet = StreamPacket::data(kind, payload);
    let envelope = StreamEnvelope::new(
        metadata.id().clone(),
        packet_id(metadata.id(), sequence),
        StreamMedia::Data,
        StreamDirection::Source,
        sequence,
        Vec::new(),
        ClockDomain::ServerFrame,
        TransportProfile::remote_stream_fabric(),
        Vec::new(),
        packet,
    )?;
    stream_chunk_frame_from_expr(cx, codec, &envelope.to_expr(), frame_envelope)
}

fn refusal_frame(
    cx: &mut Cx,
    codec: Symbol,
    metadata: &StreamMetadata,
    sequence: u64,
    message: String,
    frame_envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    let kind = server_placement_refusal_diagnostic_kind();
    let packet = diagnostic_stream_packet(kind.clone(), message);
    let envelope = StreamEnvelope::new(
        metadata.id().clone(),
        packet_id(metadata.id(), sequence),
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

fn finish_with_refusal(
    cx: &mut Cx,
    codec: Symbol,
    metadata: &StreamMetadata,
    sequence: u64,
    message: String,
    frame_envelope: FrameEnvelope,
    mut frames: Vec<ServerFrame>,
) -> Result<Vec<ServerFrame>> {
    frames.push(refusal_frame(
        cx,
        codec.clone(),
        metadata,
        sequence,
        message,
        frame_envelope.clone(),
    )?);
    frames.push(stream_end_frame(codec, frame_envelope));
    Ok(frames)
}

fn limit_cutoff_message(
    frame: &ServerFrame,
    limits: PlacementResourceLimits,
    accounted_payload_bytes: &mut usize,
    data_frame_count: usize,
    frame_count: usize,
) -> Option<String> {
    if data_frame_count >= limits.effective_stream_frame_limit() {
        return Some(format!(
            "server placement resource cutoff: stream-size limit exceeded after {data_frame_count} chunks"
        ));
    }
    if frame_count >= limits.max_inflight_work {
        return Some(format!(
            "server placement resource cutoff: inflight-work limit exceeded at {} frames",
            limits.max_inflight_work
        ));
    }
    if frame.payload.len() > limits.max_frame_payload_bytes {
        return Some(format!(
            "server placement resource cutoff: frame-size limit exceeded: {} bytes > {} bytes",
            frame.payload.len(),
            limits.max_frame_payload_bytes
        ));
    }
    let next_payload_bytes = accounted_payload_bytes.saturating_add(frame.payload.len());
    if next_payload_bytes > limits.max_memory_bytes {
        return Some(format!(
            "server placement resource cutoff: memory limit exceeded: {next_payload_bytes} bytes > {} bytes",
            limits.max_memory_bytes
        ));
    }
    *accounted_payload_bytes = next_payload_bytes;
    None
}

fn require_host_device_access(
    cx: &mut Cx,
    fragment: &PlacedFragment,
    request: &ServerPlacementRequest,
) -> Result<()> {
    if placement_uses_host_device(fragment, request) {
        cx.require(&placement_host_device_capability())?;
    }
    Ok(())
}

fn envelope_with_profile(
    envelope: &StreamEnvelope,
    profile: TransportProfile,
) -> Result<StreamEnvelope> {
    StreamEnvelope::new_with_clock_domains(
        envelope.stream_id().clone(),
        envelope.packet_id().clone(),
        envelope.media(),
        envelope.direction(),
        envelope.sequence(),
        envelope.ticks().to_vec(),
        envelope.clock_domain(),
        envelope.clock_domains().to_vec(),
        profile,
        envelope.diagnostics().to_vec(),
        envelope.packet().clone(),
    )
}

fn redacted_fragment_label(fragment: &PlacedFragment) -> String {
    redact_placement_symbol(fragment.id()).as_qualified_str()
}

fn packet_id(stream_id: &Symbol, sequence: u64) -> Symbol {
    Symbol::qualified(
        "stream/packet-id",
        format!(
            "{}#server-placement-{sequence}",
            stream_id.as_qualified_str()
        ),
    )
}
