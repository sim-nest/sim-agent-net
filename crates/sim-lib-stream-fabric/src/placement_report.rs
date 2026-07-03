use sim_kernel::{CapabilityName, Expr};
use sim_lib_stream_core::{
    LatencyClass, PlacedFragment, StreamEdge, StreamEnvelope, TransportProfile,
    stream_remote_network_capability, stream_remote_preview_capability,
};

use crate::placement::{
    ServerPlacementRequest, server_placement_request_symbol, server_placement_result_symbol,
    server_site_symbol,
};
use crate::placement_security::{
    placement_expr_uses_host_device, placement_host_device_capability,
    placement_remote_render_capability, placement_run_node_on_server_capability,
    redact_placement_expr, redact_placement_symbol,
};

pub(crate) fn request_report_expr(
    fragment: &PlacedFragment,
    request: &ServerPlacementRequest,
) -> Expr {
    Expr::Map(vec![
        field("operation", Expr::Symbol(server_placement_request_symbol())),
        field("site", Expr::Symbol(server_site_symbol())),
        field(
            "fragment-id",
            Expr::Symbol(redact_placement_symbol(fragment.id())),
        ),
        field("node", redact_placement_expr(fragment.node())),
        field("realtime-pinned", Expr::Bool(request.realtime_pinned())),
        field(
            "placement-report",
            redact_placement_expr(request.placement_report()),
        ),
        field("declared-profile", request.output_profile().to_expr()),
        field(
            "declared-latency",
            Expr::Symbol(request.output_profile().latency_class().symbol()),
        ),
        field("resource-limits", request.resource_limits().to_expr()),
        field(
            "required-capabilities",
            required_capabilities_expr(fragment, request),
        ),
        field("input-edges", edge_list_expr(fragment.input_edges())),
        field("output-edges", edge_list_expr(fragment.output_edges())),
    ])
}

pub(crate) fn result_report_expr(
    fragment: &PlacedFragment,
    payload: Expr,
    output_profile: &TransportProfile,
) -> Expr {
    Expr::Map(vec![
        field("operation", Expr::Symbol(server_placement_result_symbol())),
        field("site", Expr::Symbol(server_site_symbol())),
        field(
            "fragment-id",
            Expr::Symbol(redact_placement_symbol(fragment.id())),
        ),
        field("node-result", redact_placement_expr(&payload)),
        field(
            "output-envelope-count",
            Expr::String(fragment.output_envelopes().len().to_string()),
        ),
        field("declared-profile", output_profile.to_expr()),
        field(
            "declared-latency",
            Expr::Symbol(output_profile.latency_class().symbol()),
        ),
    ])
}

pub(crate) fn placement_uses_host_device(
    fragment: &PlacedFragment,
    request: &ServerPlacementRequest,
) -> bool {
    placement_expr_uses_host_device(fragment.node())
        || placement_expr_uses_host_device(request.placement_report())
        || fragment.input_edges().iter().any(edge_uses_host_device)
        || fragment.output_edges().iter().any(edge_uses_host_device)
}

fn edge_list_expr(edges: &[StreamEdge]) -> Expr {
    Expr::List(edges.iter().map(edge_expr).collect())
}

fn edge_expr(edge: &StreamEdge) -> Expr {
    let rate = edge.rate_contract();
    Expr::Map(vec![
        field("port", Expr::Symbol(redact_placement_symbol(edge.port()))),
        field("clock-domain", Expr::Symbol(rate.clock_domain().symbol())),
        field("latency-class", Expr::Symbol(rate.latency_class().symbol())),
        field(
            "nominal-rate-hz",
            rate.nominal_rate_hz()
                .map(|rate| Expr::String(rate.to_string()))
                .unwrap_or(Expr::Nil),
        ),
        field(
            "metadata",
            redact_placement_expr(&edge.metadata().table_expr()),
        ),
        field(
            "envelopes",
            Expr::List(
                edge.envelopes()
                    .iter()
                    .map(StreamEnvelope::to_expr)
                    .map(|expr| redact_placement_expr(&expr))
                    .collect(),
            ),
        ),
    ])
}

fn edge_uses_host_device(edge: &StreamEdge) -> bool {
    placement_expr_uses_host_device(&Expr::Symbol(edge.port().clone()))
        || placement_expr_uses_host_device(&edge.metadata().table_expr())
        || edge
            .envelopes()
            .iter()
            .any(|envelope| placement_expr_uses_host_device(&envelope.to_expr()))
}

fn required_capabilities_expr(fragment: &PlacedFragment, request: &ServerPlacementRequest) -> Expr {
    let mut capabilities = vec![
        placement_run_node_on_server_capability(),
        stream_remote_network_capability(),
    ];
    capabilities.extend(profile_capabilities(request.output_profile()));
    if placement_uses_host_device(fragment, request) {
        capabilities.push(placement_host_device_capability());
    }
    capabilities.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    capabilities.dedup();
    Expr::List(
        capabilities
            .into_iter()
            .map(|capability| Expr::String(capability.as_str().to_owned()))
            .collect(),
    )
}

fn profile_capabilities(profile: &TransportProfile) -> Vec<CapabilityName> {
    match profile.latency_class() {
        LatencyClass::BufferedPreview => vec![stream_remote_preview_capability()],
        LatencyClass::OfflineRender => vec![placement_remote_render_capability()],
        LatencyClass::Interactive => Vec::new(),
        _ => Vec::new(),
    }
}

use sim_value::build::entry as field;
