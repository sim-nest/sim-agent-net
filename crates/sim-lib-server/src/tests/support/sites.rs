use sim_kernel::{Expr, Symbol};

use crate::{EvalSite, FrameKind, ServerAddress, ServerFrame};

use super::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct LoopbackSite {
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
}

impl EvalSite for LoopbackSite {
    fn site_kind(&self) -> &'static str {
        "loopback"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        _cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        Ok(frame)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) struct ResponseSite {
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
}

impl EvalSite for ResponseSite {
    fn site_kind(&self) -> &'static str {
        "response"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        ServerFrame::from_expr(
            cx,
            frame.codec.clone(),
            FrameKind::Response,
            &Expr::Map(vec![
                (Expr::Symbol(Symbol::new("value")), Expr::Nil),
                (
                    Expr::Symbol(Symbol::new("diagnostics")),
                    Expr::List(Vec::new()),
                ),
                (Expr::Symbol(Symbol::new("trace")), Expr::Nil),
            ]),
            frame.envelope.consistency,
            Vec::new(),
            false,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) struct RecordingSite {
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) seen: Arc<Mutex<Vec<FrameKind>>>,
}

impl EvalSite for RecordingSite {
    fn site_kind(&self) -> &'static str {
        "recording"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        self.seen
            .lock()
            .expect("recording site mutex poisoned")
            .push(frame.kind.clone());
        ServerFrame::from_expr(
            cx,
            frame.codec.clone(),
            FrameKind::Response,
            &Expr::Map(vec![
                (Expr::Symbol(Symbol::new("value")), Expr::Nil),
                (
                    Expr::Symbol(Symbol::new("diagnostics")),
                    Expr::List(Vec::new()),
                ),
                (Expr::Symbol(Symbol::new("trace")), Expr::Nil),
            ]),
            frame.envelope.consistency,
            Vec::new(),
            false,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) struct TransformSite {
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) prefix: &'static str,
    pub(crate) hops: Arc<Mutex<Vec<u32>>>,
}

impl EvalSite for TransformSite {
    fn site_kind(&self) -> &'static str {
        "transform"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        self.hops
            .lock()
            .expect("transform site mutex poisoned")
            .push(frame.envelope.hop);
        let request = crate::eval_request_from_frame(cx, &frame)?;
        let Expr::String(text) = request.expr else {
            panic!("transform site expects string requests");
        };
        crate::server_frame_from_reply(
            cx,
            &frame.codec,
            sim_kernel::EvalReply {
                value: cx.factory().string(format!("{}{}", self.prefix, text))?,
                diagnostics: Vec::new(),
                trace: None,
            },
            frame.envelope.consistency,
        )
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) struct MultiChunkSite {
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
}

impl EvalSite for MultiChunkSite {
    fn site_kind(&self) -> &'static str {
        "multi-chunk"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
    ) -> sim_kernel::Result<ServerFrame> {
        ServerFrame::from_expr(
            cx,
            frame.codec.clone(),
            FrameKind::Response,
            &Expr::Map(vec![
                (Expr::Symbol(Symbol::new("value")), Expr::Nil),
                (
                    Expr::Symbol(Symbol::new("diagnostics")),
                    Expr::List(Vec::new()),
                ),
                (Expr::Symbol(Symbol::new("trace")), Expr::Nil),
            ]),
            frame.envelope.consistency,
            Vec::new(),
            false,
        )
    }

    fn stream(
        &self,
        cx: &mut sim_kernel::Cx,
        frame: ServerFrame,
        sink: &mut dyn crate::StreamSink,
    ) -> sim_kernel::Result<()> {
        sink.chunk(
            cx,
            ServerFrame::new(
                frame.codec.clone(),
                FrameKind::StreamStart,
                frame.envelope.clone(),
                Vec::new(),
            ),
        )?;
        for text in ["alpha", "beta"] {
            let chunk = ServerFrame::from_expr(
                cx,
                frame.codec.clone(),
                FrameKind::StreamChunk,
                &Expr::String(text.to_owned()),
                frame.envelope.consistency,
                Vec::new(),
                false,
            )?;
            sink.chunk(cx, chunk)?;
        }
        sink.chunk(
            cx,
            ServerFrame::new(
                frame.codec,
                FrameKind::StreamEnd,
                frame.envelope,
                Vec::new(),
            ),
        )?;
        sink.end(cx)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
