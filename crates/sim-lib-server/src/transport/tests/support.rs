use std::sync::Arc;

use sim_codec_binary::BinaryCodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Symbol};

pub(crate) fn cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    cx
}

pub(crate) fn codecs() -> Vec<Symbol> {
    vec![
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "lisp"),
        Symbol::qualified("codec", "json"),
    ]
}

#[cfg(feature = "server-net-http")]
#[derive(Default)]
pub(crate) struct CollectingSink {
    pub(crate) chunks: Vec<sim_kernel::Expr>,
    pub(crate) seen: Vec<crate::FrameKind>,
    pub(crate) ended: bool,
}

#[cfg(feature = "server-net-http")]
impl crate::StreamSink for CollectingSink {
    fn chunk(
        &mut self,
        cx: &mut sim_kernel::Cx,
        frame: crate::ServerFrame,
    ) -> sim_kernel::Result<()> {
        self.seen.push(frame.kind.clone());
        match frame.kind {
            crate::FrameKind::StreamStart => Ok(()),
            crate::FrameKind::StreamChunk => {
                self.chunks
                    .push(frame.decode_expr(cx, sim_kernel::ReadPolicy::default())?);
                Ok(())
            }
            crate::FrameKind::StreamEnd => {
                self.ended = true;
                Ok(())
            }
            other => Err(sim_kernel::Error::Eval(format!(
                "unexpected frame kind {}",
                other.as_symbol()
            ))),
        }
    }

    fn end(&mut self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<()> {
        self.ended = true;
        Ok(())
    }
}
