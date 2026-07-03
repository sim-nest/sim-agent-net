use sim_codec_mcp::{McpEnvelope, envelope_to_expr, expr_to_envelope};
use sim_kernel::{CapabilityName, Cx, Error, Expr, ReadPolicy, Result, Symbol};
use sim_lib_server::{
    FrameEnvelope, FrameKind, ServerFrame, decode_transport_frame, encode_transport_frame,
};

use crate::{McpRouter, McpSession};

/// Returns the capability gating MCP-over-HTTP transport.
pub fn mcp_http_capability() -> CapabilityName {
    CapabilityName::new("mcp.http")
}

/// Bridges MCP envelopes to and from server transport frames over HTTP, SSE,
/// and WebSocket, dispatching them through an [`McpRouter`].
pub struct McpHttpAdapter {
    router: McpRouter,
    codec: Symbol,
}

impl McpHttpAdapter {
    /// Creates an adapter routing against `session` with the MCP codec.
    pub fn new(session: McpSession) -> Self {
        Self {
            router: McpRouter::new(session),
            codec: Symbol::qualified("codec", "mcp"),
        }
    }

    /// Returns the underlying router.
    pub fn router(&self) -> &McpRouter {
        &self.router
    }

    /// Returns a mutable reference to the underlying router.
    pub fn router_mut(&mut self) -> &mut McpRouter {
        &mut self.router
    }

    /// Routes one HTTP request `envelope` and returns the single reply, if any.
    pub fn handle_http_envelope(
        &mut self,
        cx: &mut Cx,
        envelope: McpEnvelope,
    ) -> Result<Option<McpEnvelope>> {
        let frame = self.frame_from_envelope(cx, &envelope, FrameEnvelope::default())?;
        let reply = self.handle_http_frame(cx, frame)?;
        reply
            .map(|frame| self.envelope_from_frame(cx, &frame))
            .transpose()
    }

    /// Routes one SSE request `envelope` and returns every reply in order.
    pub fn handle_sse_envelope(
        &mut self,
        cx: &mut Cx,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        let frame = self.frame_from_envelope(cx, &envelope, FrameEnvelope::default())?;
        self.handle_sse_frame(cx, frame)?
            .iter()
            .map(|frame| self.envelope_from_frame(cx, frame))
            .collect()
    }

    /// Routes a batch of WebSocket `envelopes` and returns all replies in order.
    pub fn handle_websocket_envelopes(
        &mut self,
        cx: &mut Cx,
        envelopes: impl IntoIterator<Item = McpEnvelope>,
    ) -> Result<Vec<McpEnvelope>> {
        let frames = envelopes
            .into_iter()
            .map(|envelope| self.frame_from_envelope(cx, &envelope, FrameEnvelope::default()))
            .collect::<Result<Vec<_>>>()?;
        self.handle_websocket_frames(cx, frames)?
            .iter()
            .map(|frame| self.envelope_from_frame(cx, frame))
            .collect()
    }

    /// Routes one HTTP transport `frame`, returning the single reply frame, if
    /// any; requires the [`mcp_http_capability`] gate.
    pub fn handle_http_frame(
        &mut self,
        cx: &mut Cx,
        frame: ServerFrame,
    ) -> Result<Option<ServerFrame>> {
        self.require_network_gate()?;
        let envelope = self.envelope_from_frame(cx, &frame)?;
        let reply = self.router.handle(cx, envelope)?;
        reply
            .map(|reply| self.frame_from_envelope(cx, &reply, frame.envelope.clone()))
            .transpose()
    }

    /// Routes one SSE transport `frame`, returning every reply frame in order;
    /// requires the [`mcp_http_capability`] gate.
    pub fn handle_sse_frame(
        &mut self,
        cx: &mut Cx,
        frame: ServerFrame,
    ) -> Result<Vec<ServerFrame>> {
        self.require_network_gate()?;
        let envelope = self.envelope_from_frame(cx, &frame)?;
        let replies = self.router.handle_many(cx, envelope)?;
        self.replies_to_frames(cx, replies, frame.envelope)
    }

    /// Routes a batch of WebSocket transport `frames`, returning all reply
    /// frames in order; requires the [`mcp_http_capability`] gate.
    pub fn handle_websocket_frames(
        &mut self,
        cx: &mut Cx,
        frames: impl IntoIterator<Item = ServerFrame>,
    ) -> Result<Vec<ServerFrame>> {
        self.require_network_gate()?;
        let mut out = Vec::new();
        for frame in frames {
            let envelope = self.envelope_from_frame(cx, &frame)?;
            let replies = self.router.handle_many(cx, envelope)?;
            out.extend(self.replies_to_frames(cx, replies, frame.envelope)?);
        }
        Ok(out)
    }

    /// Encodes a transport `frame` to its wire bytes.
    pub fn encode_frame(frame: &ServerFrame) -> Result<Vec<u8>> {
        encode_transport_frame(frame)
    }

    /// Decodes a transport frame from wire `bytes`.
    pub fn decode_frame(bytes: &[u8]) -> Result<ServerFrame> {
        decode_transport_frame(bytes)
    }

    /// Encodes `envelope` into a transport frame carrying `envelope_meta`.
    pub fn frame_from_envelope(
        &self,
        cx: &mut Cx,
        envelope: &McpEnvelope,
        envelope_meta: FrameEnvelope,
    ) -> Result<ServerFrame> {
        self.frame_from_expr(
            cx,
            frame_kind_for_envelope(envelope),
            &envelope_to_expr(envelope),
            envelope_meta,
        )
    }

    /// Decodes the MCP envelope carried by a transport `frame`.
    pub fn envelope_from_frame(&self, cx: &mut Cx, frame: &ServerFrame) -> Result<McpEnvelope> {
        let expr = frame.decode_expr(cx, ReadPolicy::default())?;
        expr_to_envelope(&expr)
    }

    fn replies_to_frames(
        &self,
        cx: &mut Cx,
        replies: Vec<McpEnvelope>,
        envelope: FrameEnvelope,
    ) -> Result<Vec<ServerFrame>> {
        replies
            .into_iter()
            .map(|reply| self.frame_from_envelope(cx, &reply, envelope.clone()))
            .collect()
    }

    fn frame_from_expr(
        &self,
        cx: &mut Cx,
        kind: FrameKind,
        expr: &Expr,
        envelope: FrameEnvelope,
    ) -> Result<ServerFrame> {
        let mut frame = ServerFrame::from_expr(
            cx,
            self.codec.clone(),
            kind,
            expr,
            envelope.consistency,
            envelope.required_capabilities.clone(),
            envelope.trace,
        )?;
        frame.envelope = envelope;
        Ok(frame)
    }

    fn require_network_gate(&self) -> Result<()> {
        let capability = mcp_http_capability();
        if self
            .router
            .session()
            .granted_capabilities
            .iter()
            .any(|granted| granted == &capability)
        {
            Ok(())
        } else {
            Err(Error::CapabilityDenied { capability })
        }
    }
}

fn frame_kind_for_envelope(envelope: &McpEnvelope) -> FrameKind {
    match envelope {
        McpEnvelope::Request(_) => FrameKind::Request,
        McpEnvelope::Notification(_) => FrameKind::Notify,
        McpEnvelope::Response(_) => FrameKind::Response,
        McpEnvelope::Error(_) => FrameKind::Error,
    }
}
