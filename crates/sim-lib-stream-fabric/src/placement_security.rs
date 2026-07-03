use sim_kernel::{CapabilityName, Expr, Result, Symbol};
use sim_lib_stream_core::{
    StreamRedactionFinding, StreamRemoteLimits, StreamSecurityPolicy,
    stream_host_device_capability, stream_remote_render_capability,
};

/// Capability a peer must hold to place work across the stream fabric.
///
/// Each variant maps to a stable wire label and a kernel `CapabilityName`,
/// gating where a node may run (server or LAN peer), whether it may render
/// remotely, and whether it may reach a host audio/IO device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementCapability {
    /// Run a placed node on the server site.
    RunNodeOnServer,
    /// Run a placed node on a LAN peer site.
    RunNodeOnLanPeer,
    /// Render a fragment remotely and return the result.
    RemoteRender,
    /// Reach a host device (audio, `/dev`, hardware port) during placement.
    HostDevice,
}

impl PlacementCapability {
    /// Returns the stable wire label string for this capability.
    pub fn wire_label(self) -> &'static str {
        match self {
            Self::RunNodeOnServer => "stream.placement.run-node-on-server",
            Self::RunNodeOnLanPeer => "stream.placement.run-node-on-lan-peer",
            Self::RemoteRender => "stream.remote.render",
            Self::HostDevice => "stream.host.device",
        }
    }

    /// Returns the kernel capability name this placement capability requires.
    pub fn capability(self) -> CapabilityName {
        match self {
            Self::RemoteRender => stream_remote_render_capability(),
            Self::HostDevice => stream_host_device_capability(),
            Self::RunNodeOnServer | Self::RunNodeOnLanPeer => {
                CapabilityName::new(self.wire_label())
            }
        }
    }

    /// Returns the qualified symbol identifying this capability.
    pub fn symbol(self) -> Symbol {
        Symbol::qualified("stream/placement-capability", self.wire_label())
    }
}

/// Resource bounds enforced on a server placement before and during execution.
///
/// Combines compute bounds (CPU time, memory, inflight work) with the
/// frame-shaping bounds shared with `StreamRemoteLimits`. A placement that
/// would exceed any bound is refused rather than run unbounded; see
/// [`PlacementResourceLimits::validate`]. The [`Default`] impl applies
/// conservative defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementResourceLimits {
    /// Maximum wall-clock CPU time, in milliseconds, allowed for the node.
    pub max_cpu_time_ms: u64,
    /// Maximum accounted payload memory, in bytes, across emitted frames.
    pub max_memory_bytes: usize,
    /// Maximum encoded payload size, in bytes, allowed for a single frame.
    pub max_frame_payload_bytes: usize,
    /// Maximum number of data frames emitted for one placement.
    pub max_stream_frames: usize,
    /// Maximum stream duration, in milliseconds, before truncation.
    pub max_duration_ms: u64,
    /// Maximum stream rate, in hertz, used to derive the effective frame limit.
    pub max_rate_hz: u32,
    /// Maximum number of frames allowed in flight at once.
    pub max_inflight_work: usize,
}

impl Default for PlacementResourceLimits {
    fn default() -> Self {
        let limits = StreamRemoteLimits::default();
        Self {
            max_cpu_time_ms: 5_000,
            max_memory_bytes: 64 * 1024 * 1024,
            max_frame_payload_bytes: limits.max_frame_payload_bytes,
            max_stream_frames: limits.max_stream_frames,
            max_duration_ms: limits.max_duration_ms,
            max_rate_hz: limits.max_rate_hz,
            max_inflight_work: limits.max_inflight_frames,
        }
    }
}

impl PlacementResourceLimits {
    /// Validates that every bound is positive and frame-consistent.
    ///
    /// Returns an error when CPU time, memory, stream-size, or inflight-work is
    /// zero, or when the derived [`PlacementResourceLimits::remote_limits`] are
    /// themselves invalid.
    pub fn validate(self) -> Result<()> {
        if self.max_cpu_time_ms == 0 {
            return Err(sim_kernel::Error::Eval(
                "placement cpu-time limit must be positive".to_owned(),
            ));
        }
        if self.max_memory_bytes == 0 {
            return Err(sim_kernel::Error::Eval(
                "placement memory limit must be positive".to_owned(),
            ));
        }
        if self.max_stream_frames == 0 {
            return Err(sim_kernel::Error::Eval(
                "placement stream-size limit must be positive".to_owned(),
            ));
        }
        if self.max_inflight_work == 0 {
            return Err(sim_kernel::Error::Eval(
                "placement inflight-work limit must be positive".to_owned(),
            ));
        }
        self.remote_limits().validate()
    }

    /// Projects the frame-shaping subset of these limits onto a
    /// `StreamRemoteLimits`, mapping inflight-work to the inflight-frame bound.
    pub fn remote_limits(self) -> StreamRemoteLimits {
        StreamRemoteLimits {
            max_frame_payload_bytes: self.max_frame_payload_bytes,
            max_stream_frames: self.max_stream_frames,
            max_inflight_frames: self.max_inflight_work,
            max_duration_ms: self.max_duration_ms,
            max_rate_hz: self.max_rate_hz,
            max_binary_payload_bytes: StreamRemoteLimits::default().max_binary_payload_bytes,
        }
    }

    /// Returns the effective per-stream frame limit after duration and rate
    /// bounds are folded into the configured stream-size limit.
    pub fn effective_stream_frame_limit(self) -> usize {
        self.remote_limits().effective_frame_limit()
    }

    /// Encodes these limits as a string-valued map `Expr` for placement reports.
    pub fn to_expr(self) -> Expr {
        Expr::Map(vec![
            field("max-cpu-time-ms", self.max_cpu_time_ms.to_string()),
            field("max-memory-bytes", self.max_memory_bytes.to_string()),
            field(
                "max-frame-payload-bytes",
                self.max_frame_payload_bytes.to_string(),
            ),
            field("max-stream-frames", self.max_stream_frames.to_string()),
            field("max-duration-ms", self.max_duration_ms.to_string()),
            field("max-rate-hz", self.max_rate_hz.to_string()),
            field("max-inflight-work", self.max_inflight_work.to_string()),
        ])
    }
}

/// Returns the capability required to run a placed node on the server site.
pub fn placement_run_node_on_server_capability() -> CapabilityName {
    PlacementCapability::RunNodeOnServer.capability()
}

/// Returns the capability required to run a placed node on a LAN peer site.
pub fn placement_run_node_on_lan_peer_capability() -> CapabilityName {
    PlacementCapability::RunNodeOnLanPeer.capability()
}

/// Returns the capability required to render a fragment remotely.
pub fn placement_remote_render_capability() -> CapabilityName {
    PlacementCapability::RemoteRender.capability()
}

/// Returns the capability required to reach a host device during placement.
pub fn placement_host_device_capability() -> CapabilityName {
    PlacementCapability::HostDevice.capability()
}

/// Returns the kernel capability names for every [`PlacementCapability`].
pub fn placement_capability_names() -> Vec<CapabilityName> {
    [
        PlacementCapability::RunNodeOnServer,
        PlacementCapability::RunNodeOnLanPeer,
        PlacementCapability::RemoteRender,
        PlacementCapability::HostDevice,
    ]
    .into_iter()
    .map(PlacementCapability::capability)
    .collect()
}

/// Returns a copy of `expr` with sensitive placement data redacted.
///
/// Walks the expression under the default `StreamSecurityPolicy`, replacing
/// flagged strings, symbols, large binary payloads, and host-device references
/// with redaction placeholders so placement reports can be safely transported.
pub fn redact_placement_expr(expr: &Expr) -> Expr {
    redact_expr(expr, StreamSecurityPolicy::default())
}

/// Returns `symbol`, or a redaction placeholder if it names sensitive data.
///
/// A symbol is redacted when the default security policy flags its qualified
/// text or when it references a host device.
pub fn redact_placement_symbol(symbol: &Symbol) -> Symbol {
    if StreamSecurityPolicy::default()
        .finding_for_text(&symbol.as_qualified_str())
        .is_some()
        || placement_text_uses_host_device(&symbol.as_qualified_str())
    {
        Symbol::qualified("stream/redacted", "placement")
    } else {
        symbol.clone()
    }
}

/// Returns whether any symbol, string, or nested node in `expr` references a
/// host device.
pub fn placement_expr_uses_host_device(expr: &Expr) -> bool {
    match expr {
        Expr::Symbol(symbol) | Expr::Local(symbol) => {
            placement_text_uses_host_device(&symbol.as_qualified_str())
        }
        Expr::String(value) => placement_text_uses_host_device(value),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            items.iter().any(placement_expr_uses_host_device)
        }
        Expr::Map(entries) => entries.iter().any(|(key, value)| {
            placement_expr_uses_host_device(key) || placement_expr_uses_host_device(value)
        }),
        Expr::Call { operator, args } => {
            placement_expr_uses_host_device(operator)
                || args.iter().any(placement_expr_uses_host_device)
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => {
            placement_text_uses_host_device(&operator.as_qualified_str())
                || placement_expr_uses_host_device(left)
                || placement_expr_uses_host_device(right)
        }
        Expr::Prefix { operator, arg } | Expr::Postfix { operator, arg } => {
            placement_text_uses_host_device(&operator.as_qualified_str())
                || placement_expr_uses_host_device(arg)
        }
        Expr::Quote { expr, .. } => placement_expr_uses_host_device(expr),
        Expr::Annotated { expr, annotations } => {
            placement_expr_uses_host_device(expr)
                || annotations.iter().any(|(key, value)| {
                    placement_text_uses_host_device(&key.as_qualified_str())
                        || placement_expr_uses_host_device(value)
                })
        }
        Expr::Extension { tag, payload } => {
            placement_text_uses_host_device(&tag.as_qualified_str())
                || placement_expr_uses_host_device(payload)
        }
        _ => false,
    }
}

/// Returns whether `value` mentions a host-device path, driver, or prefix.
pub fn placement_text_uses_host_device(value: &str) -> bool {
    value.contains("/dev/")
        || value.contains("hw:")
        || value.contains("CoreAudio")
        || value.contains("ALSA")
        || value.contains("host-device")
        || value.starts_with("device/")
        || value.starts_with("host/device")
}

fn redact_expr(expr: &Expr, policy: StreamSecurityPolicy) -> Expr {
    match expr {
        Expr::Symbol(symbol) => Expr::Symbol(redact_placement_symbol(symbol)),
        Expr::Local(symbol) => Expr::Local(redact_placement_symbol(symbol)),
        Expr::String(value) => Expr::String(redact_text(value, policy)),
        Expr::Bytes(bytes)
            if policy.finding_for_expr(expr) == Some(StreamRedactionFinding::LargeBinaryData) =>
        {
            Expr::String("[redacted placement payload]".to_owned())
        }
        Expr::List(items) => {
            Expr::List(items.iter().map(|item| redact_expr(item, policy)).collect())
        }
        Expr::Vector(items) => {
            Expr::Vector(items.iter().map(|item| redact_expr(item, policy)).collect())
        }
        Expr::Set(items) => Expr::Set(items.iter().map(|item| redact_expr(item, policy)).collect()),
        Expr::Map(entries) => Expr::Map(
            entries
                .iter()
                .map(|(key, value)| (redact_expr(key, policy), redact_expr(value, policy)))
                .collect(),
        ),
        Expr::Call { operator, args } => Expr::Call {
            operator: Box::new(redact_expr(operator, policy)),
            args: args.iter().map(|arg| redact_expr(arg, policy)).collect(),
        },
        Expr::Infix {
            operator,
            left,
            right,
        } => Expr::Infix {
            operator: redact_placement_symbol(operator),
            left: Box::new(redact_expr(left, policy)),
            right: Box::new(redact_expr(right, policy)),
        },
        Expr::Prefix { operator, arg } => Expr::Prefix {
            operator: redact_placement_symbol(operator),
            arg: Box::new(redact_expr(arg, policy)),
        },
        Expr::Postfix { operator, arg } => Expr::Postfix {
            operator: redact_placement_symbol(operator),
            arg: Box::new(redact_expr(arg, policy)),
        },
        Expr::Block(items) => {
            Expr::Block(items.iter().map(|item| redact_expr(item, policy)).collect())
        }
        Expr::Quote { mode, expr } => Expr::Quote {
            mode: *mode,
            expr: Box::new(redact_expr(expr, policy)),
        },
        Expr::Annotated { expr, annotations } => Expr::Annotated {
            expr: Box::new(redact_expr(expr, policy)),
            annotations: annotations
                .iter()
                .map(|(key, value)| (redact_placement_symbol(key), redact_expr(value, policy)))
                .collect(),
        },
        Expr::Extension { tag, payload } => Expr::Extension {
            tag: redact_placement_symbol(tag),
            payload: Box::new(redact_expr(payload, policy)),
        },
        other => other.clone(),
    }
}

fn redact_text(value: &str, policy: StreamSecurityPolicy) -> String {
    if policy.finding_for_text(value).is_some() || placement_text_uses_host_device(value) {
        "[redacted placement data]".to_owned()
    } else {
        value.to_owned()
    }
}

fn field(name: &str, value: String) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), Expr::String(value))
}
