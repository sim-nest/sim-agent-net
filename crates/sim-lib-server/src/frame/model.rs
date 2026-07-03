use std::time::Duration;

use sim_codec::DecodeLimits;
use sim_kernel::{
    CapabilityName, ClassRef, Consistency, Cx, EncodeOptions, Expr, Object, ReadPolicy, Result,
    Symbol, Value,
};

use crate::codecio::{decode_frame_payload, encode_frame_payload};
use crate::helpers::format_duration;

const SERVER_FRAME_VERSION: u16 = 1;

/// Discriminates the role a [`ServerFrame`] plays in the wire protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// An eval/agent request.
    Request,
    /// A successful response to a request.
    Response,
    /// An error response to a request.
    Error,
    /// A one-way notification with no reply expected.
    Notify,
    /// The opening frame of a streamed response.
    StreamStart,
    /// One chunk of a streamed response.
    StreamChunk,
    /// The closing frame of a streamed response.
    StreamEnd,
    /// A codec negotiation offer.
    Negotiate {
        /// Codecs the sender is willing to use.
        codecs: Vec<Symbol>,
    },
    /// A liveness probe.
    Ping,
    /// A reply to a [`FrameKind::Ping`].
    Pong,
    /// A lifecycle control message.
    Lifecycle {
        /// Lifecycle command being issued.
        command: LifecycleCommand,
    },
    /// A scheduled or event-driven trigger.
    Trigger {
        /// Symbol naming the trigger source.
        source: Symbol,
        /// Trigger time in milliseconds.
        when_ms: u64,
    },
    /// A role assignment for multi-hop routing.
    Role {
        /// Symbol naming the assigned role.
        role: Symbol,
        /// Hop count at which the role applies.
        hop: u32,
    },
}

impl FrameKind {
    /// Returns the symbol naming this frame kind (e.g. `request`, `stream-end`).
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Error => "error",
            Self::Notify => "notify",
            Self::StreamStart => "stream-start",
            Self::StreamChunk => "stream-chunk",
            Self::StreamEnd => "stream-end",
            Self::Negotiate { .. } => "negotiate",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::Lifecycle { .. } => "lifecycle",
            Self::Trigger { .. } => "trigger",
            Self::Role { .. } => "role",
        })
    }
}

/// Command carried by a [`FrameKind::Lifecycle`] frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleCommand {
    /// Start the server.
    Start,
    /// Stop the server.
    Stop,
    /// Suspend serving without tearing down.
    Suspend,
    /// Resume a suspended server.
    Resume,
    /// Request a health report.
    Health,
}

/// Out-of-band metadata attached to a [`ServerFrame`] alongside its payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameEnvelope {
    /// Optional deadline for handling the frame.
    pub deadline: Option<Duration>,
    /// Consistency policy requested for the call.
    pub consistency: Consistency,
    /// Whether an execution trace is requested.
    pub trace: bool,
    /// Capabilities the call requires.
    pub required_capabilities: Vec<CapabilityName>,
    /// Codec the sender prefers for the reply, if any.
    pub reply_codec_hint: Option<Symbol>,
    /// Routing role assigned to the frame, if any.
    pub role: Option<Symbol>,
    /// Hop count for multi-hop routing.
    pub hop: u32,
    /// Symbol naming the trigger source, if the frame was triggered.
    pub trigger_source: Option<Symbol>,
}

impl FrameEnvelope {
    fn as_value(&self, cx: &mut Cx) -> Result<Value> {
        let deadline = match self.deadline {
            Some(deadline) => cx.factory().string(format_duration(deadline))?,
            None => cx.factory().nil()?,
        };
        let required_capabilities = cx.factory().list(
            self.required_capabilities
                .iter()
                .map(|capability| cx.factory().string(capability.as_str().to_owned()))
                .collect::<Result<Vec<_>>>()?,
        )?;
        let reply_codec_hint = match &self.reply_codec_hint {
            Some(codec) => cx.factory().symbol(codec.clone())?,
            None => cx.factory().nil()?,
        };
        let role = match &self.role {
            Some(role) => cx.factory().symbol(role.clone())?,
            None => cx.factory().nil()?,
        };
        let trigger_source = match &self.trigger_source {
            Some(source) => cx.factory().symbol(source.clone())?,
            None => cx.factory().nil()?,
        };
        cx.factory().table(vec![
            (Symbol::new("deadline"), deadline),
            (
                Symbol::new("consistency"),
                cx.factory().symbol(self.consistency.as_symbol())?,
            ),
            (Symbol::new("trace"), cx.factory().bool(self.trace)?),
            (Symbol::new("requires"), required_capabilities),
            (Symbol::new("reply-codec-hint"), reply_codec_hint),
            (Symbol::new("role"), role),
            (
                Symbol::new("hop"),
                cx.factory().string(self.hop.to_string())?,
            ),
            (Symbol::new("trigger-source"), trigger_source),
        ])
    }
}

#[sim_citizen_derive::non_citizen(
    reason = "server frame runtime shell; class-backed descriptor is server/Frame",
    kind = "marker"
)]
/// A wire frame carrying a codec-encoded payload plus protocol metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFrame {
    /// Server frame format version.
    pub version: u16,
    /// Codec used to encode the payload.
    pub codec: Symbol,
    /// Identifier of this message, if assigned.
    pub msg_id: Option<u64>,
    /// Message id this frame correlates to, if it is a reply.
    pub correlate: Option<u64>,
    /// Role this frame plays in the protocol.
    pub kind: FrameKind,
    /// Out-of-band envelope metadata.
    pub envelope: FrameEnvelope,
    /// Codec-encoded payload bytes.
    pub payload: Vec<u8>,
}

impl ServerFrame {
    /// Builds a frame at the current version with no message or correlation id.
    pub fn new(codec: Symbol, kind: FrameKind, envelope: FrameEnvelope, payload: Vec<u8>) -> Self {
        Self {
            version: SERVER_FRAME_VERSION,
            codec,
            msg_id: None,
            correlate: None,
            kind,
            envelope,
            payload,
        }
    }

    /// Builds a frame by encoding `expr` under `codec` into the payload.
    ///
    /// Seeds the envelope with the given consistency, required capabilities,
    /// and trace flag, leaving the remaining envelope fields at their defaults.
    pub fn from_expr(
        cx: &mut Cx,
        codec: Symbol,
        kind: FrameKind,
        expr: &Expr,
        consistency: Consistency,
        required_capabilities: Vec<CapabilityName>,
        trace: bool,
    ) -> Result<Self> {
        let payload = encode_frame_payload(cx, &codec, expr, EncodeOptions::default())?;
        Ok(Self::new(
            codec,
            kind,
            FrameEnvelope {
                consistency,
                required_capabilities,
                trace,
                ..FrameEnvelope::default()
            },
            payload,
        ))
    }

    /// Decodes the payload back into an expression using the frame's codec.
    pub fn decode_expr(&self, cx: &mut Cx, read_policy: ReadPolicy) -> Result<Expr> {
        decode_frame_payload(
            cx,
            &self.codec,
            &self.payload,
            read_policy,
            DecodeLimits::default(),
        )
    }
}

impl Object for ServerFrame {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<server-frame {}>", self.kind.as_symbol()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for ServerFrame {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("server", "ServerFrame"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        let msg_id = match self.msg_id {
            Some(msg_id) => cx.factory().string(msg_id.to_string())?,
            None => cx.factory().nil()?,
        };
        let correlate = match self.correlate {
            Some(correlate) => cx.factory().string(correlate.to_string())?,
            None => cx.factory().nil()?,
        };
        let envelope = self.envelope.as_value(cx)?;
        cx.factory().table(vec![
            (
                Symbol::new("version"),
                cx.factory().string(self.version.to_string())?,
            ),
            (
                Symbol::new("codec"),
                cx.factory().symbol(self.codec.clone())?,
            ),
            (Symbol::new("msg-id"), msg_id),
            (Symbol::new("correlate"), correlate),
            (
                Symbol::new("kind"),
                cx.factory().symbol(self.kind.as_symbol())?,
            ),
            (Symbol::new("envelope"), envelope),
            (
                Symbol::new("payload"),
                cx.factory().bytes(self.payload.clone())?,
            ),
        ])
    }
}
