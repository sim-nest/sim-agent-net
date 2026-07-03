use std::{any::Any, time::Duration};

use sim_kernel::{Consistency, Cx, EvalFabric, EvalMode, EvalReply, EvalRequest, Result, Symbol};
use sim_lib_stream_core::{
    ClockDomain, LatencyClass, PlacedFragment, StreamEndpoint, StreamEndpointKind, StreamEnvelope,
};

use crate::helpers::default_server_codec;
use crate::{ServerAddress, ServerFrame, eval_reply_from_frame, server_frame_from_request};

/// Receiver for streamed server frames produced by [`EvalSite::stream`].
pub trait StreamSink {
    /// Accept one streamed server frame.
    ///
    /// The default `EvalSite::stream` compatibility path may pass a
    /// `FrameKind::Response` reply through this method. Real streaming sites
    /// should emit `FrameKind::StreamStart`, `FrameKind::StreamChunk`, and
    /// `FrameKind::StreamEnd` frames instead.
    fn chunk(&mut self, cx: &mut Cx, frame: ServerFrame) -> Result<()>;
    /// Finish the transport or sink.
    ///
    /// Implementations should treat this as idempotent because a
    /// `FrameKind::StreamEnd` frame may already have closed the logical stream.
    fn end(&mut self, cx: &mut Cx) -> Result<()>;
}

/// A request-answering eval endpoint: it accepts server frames and produces
/// reply frames, optionally streaming or exposing an [`EvalFabric`].
pub trait EvalSite: Send + Sync {
    /// Returns a short label naming the kind of site.
    fn site_kind(&self) -> &'static str;
    /// Returns the address this site answers at.
    fn address(&self) -> &ServerAddress;
    /// Returns the codecs this site can decode and encode.
    fn codecs(&self) -> &[Symbol];
    /// Answers `frame`, returning the reply frame.
    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame>;
    /// Answers `frame` with an optional deadline. Defaults to [`EvalSite::answer`].
    fn answer_with_timeout(
        &self,
        cx: &mut Cx,
        frame: ServerFrame,
        _timeout: Option<Duration>,
    ) -> Result<ServerFrame> {
        self.answer(cx, frame)
    }
    /// Closes any connection backing the site. Defaults to a no-op.
    fn close_connection(&self, _cx: &mut Cx) -> Result<()> {
        Ok(())
    }
    /// Returns the site as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Streams the answer to `frame` into `sink`. The default path answers once
    /// and emits a single chunk followed by `end`.
    fn stream(&self, cx: &mut Cx, frame: ServerFrame, sink: &mut dyn StreamSink) -> Result<()> {
        let reply = self.answer(cx, frame)?;
        sink.chunk(cx, reply)?;
        sink.end(cx)
    }

    /// Returns this site as an [`EvalFabric`] when it backs one. Defaults to `None`.
    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        None
    }
}

/// Identifies the variety of an eval [`Site`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteKind {
    /// Evaluates requests in the local runtime.
    Local,
    /// Resumes a coroutine for each request.
    Coroutine,
    /// Chains requests through a sequence of connection steps.
    Pipeline,
    /// Repeats a pipeline until an until-condition fires.
    Loop,
    /// Delegates to a distributed eval fabric.
    Fabric,
}

impl SiteKind {
    /// Returns the lowercase string label for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Coroutine => "coroutine",
            Self::Pipeline => "pipeline",
            Self::Loop => "loop",
            Self::Fabric => "fabric",
        }
    }

    /// Returns the `server/site`-qualified symbol naming this kind.
    pub fn symbol(self) -> Symbol {
        Symbol::qualified("server/site", self.as_str())
    }
}

/// An [`EvalSite`] that is also a stream endpoint and can realize placed
/// dataflow fragments.
pub trait Site: EvalSite + StreamEndpoint {
    /// Returns the [`SiteKind`] of this site.
    fn kind(&self) -> SiteKind;

    /// Realizes `fragment`: feeds its input edges, evaluates its node, and emits
    /// a result envelope per output edge.
    fn run_fragment(&self, cx: &mut Cx, fragment: &PlacedFragment) -> Result<Vec<StreamEnvelope>> {
        self.accept_input_edges(fragment.input_edges())?;
        let reply = self.realize_fragment_node(cx, fragment)?;
        let payload = reply.value.object().as_expr(cx)?;

        if fragment.output_edges().is_empty() {
            return self.output_envelopes(fragment);
        }

        fragment
            .output_edges()
            .iter()
            .enumerate()
            .map(|(sequence, edge)| edge.result_envelope(sequence as u64, payload.clone()))
            .collect()
    }

    /// Evaluates the fragment's node and returns its reply, with no deadline.
    fn realize_fragment_node(&self, cx: &mut Cx, fragment: &PlacedFragment) -> Result<EvalReply> {
        self.realize_fragment_node_with_timeout(cx, fragment, None)
    }

    /// Evaluates the fragment's node and returns its reply, honoring `timeout`.
    fn realize_fragment_node_with_timeout(
        &self,
        cx: &mut Cx,
        fragment: &PlacedFragment,
        timeout: Option<Duration>,
    ) -> Result<EvalReply> {
        let request = EvalRequest {
            expr: fragment.node().clone(),
            result_shape: None,
            required_capabilities: Vec::new(),
            deadline: timeout,
            consistency: Consistency::LocalFirst,
            mode: EvalMode::Eval,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            trace: false,
        };
        let codec = default_server_codec(self.codecs())?;
        let frame = server_frame_from_request(cx, &codec, request)?;
        let reply = self.answer_with_timeout(cx, frame, timeout)?;
        eval_reply_from_frame(cx, &reply)
    }
}

pub(crate) fn reply_codec_for_frame(site: &dyn EvalSite, frame: &ServerFrame) -> Symbol {
    if let Some(hint) = &frame.envelope.reply_codec_hint
        && site.codecs().iter().any(|codec| codec == hint)
    {
        return hint.clone();
    }
    default_server_codec(site.codecs()).unwrap_or_else(|_| frame.codec.clone())
}

pub(crate) fn site_endpoint_id(kind: SiteKind, address: &ServerAddress) -> Symbol {
    Symbol::qualified(
        "server/site-endpoint",
        format!("{}-{}", kind.as_str(), address.kind_symbol().name),
    )
}

pub(crate) fn eval_site_endpoint_kind() -> StreamEndpointKind {
    StreamEndpointKind::EvalSite
}

pub(crate) fn eval_site_clock_domain() -> ClockDomain {
    ClockDomain::Control
}

pub(crate) fn eval_site_latency_class() -> LatencyClass {
    LatencyClass::Interactive
}
