use std::any::Any;

use sim_kernel::{Args, Cx, EvalReply, EvalRequest, ObjectCompat, Result, Symbol, Value};
use sim_lib_server::{
    EvalSite, ServerAddress, ServerFrame, eval_request_from_frame, server_frame_from_reply,
};

/// Adapter from an opaque registry `site` value to the agent `EvalSite` surface.
#[derive(Clone)]
pub(crate) struct LoadedSite {
    value: Value,
    codecs: Vec<Symbol>,
    address: ServerAddress,
}

impl LoadedSite {
    pub(crate) fn new(value: Value, codecs: Vec<Symbol>) -> Self {
        Self {
            value,
            codecs,
            address: ServerAddress::Local,
        }
    }

    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        if let Some(fabric) = self.value.object().as_eval_fabric() {
            return fabric.realize(cx, request);
        }
        let request_expr = request.as_expr(cx)?;
        let request_value = cx.factory().expr(request_expr)?;
        let value = cx.call_value(self.value.clone(), Args::new(vec![request_value]))?;
        Ok(EvalReply {
            value,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

impl EvalSite for LoadedSite {
    fn site_kind(&self) -> &'static str {
        "loaded"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let reply_codec = frame
            .envelope
            .reply_codec_hint
            .clone()
            .unwrap_or_else(|| frame.codec.clone());
        let consistency = frame.envelope.consistency;
        let request = eval_request_from_frame(cx, &frame)?;
        let reply = self.realize(cx, request)?;
        server_frame_from_reply(cx, &reply_codec, reply, consistency)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
