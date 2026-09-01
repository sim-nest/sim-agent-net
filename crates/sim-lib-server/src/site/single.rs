use std::sync::Arc;

use sim_kernel::{Cx, Error, EvalFabric, EvalReply, ReadPolicy, Result, Symbol};
use sim_lib_stream_core::{ClockDomain, LatencyClass, StreamEndpoint, StreamEndpointKind};

use crate::{
    Coroutine, FrameKind, ServerAddress, ServerFrame, decode_frame_payload,
    eval_request_from_frame, server_frame_from_reply,
};

use super::core::{
    EvalSite, Site, SiteKind, eval_site_clock_domain, eval_site_endpoint_kind,
    eval_site_latency_class, reply_codec_for_frame, site_endpoint_id,
};
use super::pipeline::{enforce_trigger_eval_policy, eval_request_from_trigger_frame};

/// An eval site that evaluates requests directly in the local runtime.
#[derive(Clone)]
pub struct LocalEvalSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
}

impl LocalEvalSite {
    /// Creates a local site answering at `address` over `codecs`.
    pub fn new(address: ServerAddress, codecs: Vec<Symbol>) -> Self {
        Self { address, codecs }
    }
}

impl EvalSite for LocalEvalSite {
    fn site_kind(&self) -> &'static str {
        "local"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        match frame.kind {
            FrameKind::Request => {
                let consistency = frame.envelope.consistency;
                let reply_codec = reply_codec_for_frame(self, &frame);
                let request = eval_request_from_frame(cx, &frame)?;
                let reply = realize_locally(cx, request)?;
                server_frame_from_reply(cx, &reply_codec, reply, consistency)
            }
            FrameKind::Trigger { .. } => {
                realize_trigger_locally(cx, &frame)?;
                Ok(frame)
            }
            FrameKind::Notify => {
                realize_notify_locally(cx, &frame)?;
                Ok(frame)
            }
            _ => Err(Error::Eval(format!(
                "local eval site cannot answer frame kind {}",
                frame.kind.as_symbol()
            ))),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StreamEndpoint for LocalEvalSite {
    fn endpoint_id(&self) -> Symbol {
        site_endpoint_id(SiteKind::Local, &self.address)
    }

    fn endpoint_kind(&self) -> StreamEndpointKind {
        eval_site_endpoint_kind()
    }

    fn clock_domain(&self) -> ClockDomain {
        eval_site_clock_domain()
    }

    fn latency_class(&self) -> LatencyClass {
        eval_site_latency_class()
    }
}

impl Site for LocalEvalSite {
    fn kind(&self) -> SiteKind {
        SiteKind::Local
    }
}

/// An eval site that delegates requests to a distributed [`EvalFabric`].
#[derive(Clone)]
pub struct FabricEvalSite {
    kind: &'static str,
    address: ServerAddress,
    codecs: Vec<Symbol>,
    fabric: Arc<dyn EvalFabric>,
}

impl FabricEvalSite {
    /// Creates a fabric site labeled `kind`, answering at `address` over
    /// `codecs`, backed by `fabric`.
    pub fn new(
        kind: &'static str,
        address: ServerAddress,
        codecs: Vec<Symbol>,
        fabric: Arc<dyn EvalFabric>,
    ) -> Self {
        Self {
            kind,
            address,
            codecs,
            fabric,
        }
    }
}

impl EvalSite for FabricEvalSite {
    fn site_kind(&self) -> &'static str {
        self.kind
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        match frame.kind {
            FrameKind::Request => {
                let consistency = frame.envelope.consistency;
                let reply_codec = reply_codec_for_frame(self, &frame);
                let request = eval_request_from_frame(cx, &frame)?;
                let reply = self.fabric.realize(cx, request)?;
                server_frame_from_reply(cx, &reply_codec, reply, consistency)
            }
            FrameKind::Trigger { .. } => {
                let request = eval_request_from_trigger_frame(cx, &frame)?;
                let _ = self.fabric.realize(cx, request)?;
                Ok(frame)
            }
            FrameKind::StreamStart | FrameKind::StreamChunk | FrameKind::StreamEnd => Ok(frame),
            _ => Err(Error::Eval(format!(
                "fabric eval site cannot answer frame kind {}",
                frame.kind.as_symbol()
            ))),
        }
    }

    fn as_eval_fabric(&self) -> Option<&dyn EvalFabric> {
        Some(self.fabric.as_ref())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StreamEndpoint for FabricEvalSite {
    fn endpoint_id(&self) -> Symbol {
        site_endpoint_id(SiteKind::Fabric, &self.address)
    }

    fn endpoint_kind(&self) -> StreamEndpointKind {
        eval_site_endpoint_kind()
    }

    fn clock_domain(&self) -> ClockDomain {
        eval_site_clock_domain()
    }

    fn latency_class(&self) -> LatencyClass {
        eval_site_latency_class()
    }
}

impl Site for FabricEvalSite {
    fn kind(&self) -> SiteKind {
        SiteKind::Fabric
    }
}

/// An eval site that resumes a [`Coroutine`] with each request's expression.
#[derive(Clone)]
pub struct CoroutineEvalSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
    coroutine: Arc<Coroutine>,
}

impl CoroutineEvalSite {
    /// Creates a coroutine site answering at `address` over `codecs`, resuming
    /// `coroutine`.
    pub fn new(address: ServerAddress, codecs: Vec<Symbol>, coroutine: Arc<Coroutine>) -> Self {
        Self {
            address,
            codecs,
            coroutine,
        }
    }

    /// Returns the coroutine this site resumes.
    pub fn coroutine(&self) -> &Arc<Coroutine> {
        &self.coroutine
    }
}

impl EvalSite for CoroutineEvalSite {
    fn site_kind(&self) -> &'static str {
        "coroutine"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        match frame.kind {
            FrameKind::Request => {
                let consistency = frame.envelope.consistency;
                let reply_codec = reply_codec_for_frame(self, &frame);
                let request = eval_request_from_frame(cx, &frame)?;
                let input = cx.factory().expr(request.expr)?;
                let reply = self.coroutine.resume(cx, input)?;
                let diagnostics = cx.take_diagnostics();
                server_frame_from_reply(
                    cx,
                    &reply_codec,
                    EvalReply {
                        value: reply,
                        diagnostics,
                        trace: None,
                    },
                    consistency,
                )
            }
            FrameKind::Trigger { .. } => {
                let expr = frame.decode_expr(cx, ReadPolicy::default())?;
                enforce_trigger_eval_policy(cx, &expr)?;
                let input = cx.factory().expr(expr)?;
                let _ = self.coroutine.resume(cx, input)?;
                Ok(frame)
            }
            FrameKind::Notify => {
                let expr = decode_frame_payload(
                    cx,
                    &frame.codec,
                    &frame.payload,
                    ReadPolicy::default(),
                    Default::default(),
                )?;
                let input = cx.factory().expr(expr)?;
                let _ = self.coroutine.resume(cx, input)?;
                Ok(frame)
            }
            _ => Err(Error::Eval(format!(
                "coroutine eval site cannot answer frame kind {}",
                frame.kind.as_symbol()
            ))),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StreamEndpoint for CoroutineEvalSite {
    fn endpoint_id(&self) -> Symbol {
        site_endpoint_id(SiteKind::Coroutine, &self.address)
    }

    fn endpoint_kind(&self) -> StreamEndpointKind {
        eval_site_endpoint_kind()
    }

    fn clock_domain(&self) -> ClockDomain {
        eval_site_clock_domain()
    }

    fn latency_class(&self) -> LatencyClass {
        eval_site_latency_class()
    }
}

impl Site for CoroutineEvalSite {
    fn kind(&self) -> SiteKind {
        SiteKind::Coroutine
    }
}

fn realize_locally(cx: &mut Cx, request: sim_kernel::EvalRequest) -> Result<EvalReply> {
    for capability in &request.required_capabilities {
        cx.require(capability)?;
    }
    let value = cx.eval_expr(request.expr)?;
    if let Some(shape) = &request.result_shape {
        let Some(shape_object) = shape.object().as_shape() else {
            return Err(Error::TypeMismatch {
                expected: "shape",
                found: "non-shape",
            });
        };
        let matched = shape_object.check_value(cx, value.clone())?;
        if !matched.accepted {
            return Err(Error::WrongShape {
                expected: shape_object.id().unwrap_or(sim_kernel::ShapeId(0)),
                diagnostics: matched.diagnostics,
            });
        }
    }
    Ok(EvalReply {
        value,
        diagnostics: cx.take_diagnostics(),
        trace: request
            .trace
            .then(|| cx.factory().symbol(Symbol::new("local")).ok())
            .flatten(),
    })
}

fn realize_notify_locally(cx: &mut Cx, frame: &ServerFrame) -> Result<()> {
    for capability in &frame.envelope.required_capabilities {
        cx.require(capability)?;
    }
    let expr = decode_frame_payload(
        cx,
        &frame.codec,
        &frame.payload,
        ReadPolicy::default(),
        Default::default(),
    )?;
    cx.eval_expr(expr)?;
    Ok(())
}

fn realize_trigger_locally(cx: &mut Cx, frame: &ServerFrame) -> Result<()> {
    for capability in &frame.envelope.required_capabilities {
        cx.require(capability)?;
    }
    let expr = frame.decode_expr(cx, ReadPolicy::default())?;
    enforce_trigger_eval_policy(cx, &expr)?;
    cx.eval_expr(expr)?;
    Ok(())
}
