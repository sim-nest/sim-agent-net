use std::sync::{Arc, Mutex};
use std::time::Duration;

use sim_kernel::{Cx, Error, Result, Symbol};

use crate::{EvalSite, ServerAddress, helpers::format_duration};

use super::framing::{negotiate_codec, select_default_codec};
#[cfg(feature = "server-net-http")]
use super::sse_transport::sse_stream_request;
use super::{ConnectionTransport, open_connection_transport, transport_kind};

pub struct TransportEvalSite {
    kind: &'static str,
    address: ServerAddress,
    codecs: Vec<Symbol>,
    transport: Mutex<Box<dyn ConnectionTransport>>,
}

impl TransportEvalSite {
    pub fn connect_with_loopback(
        cx: &mut Cx,
        address: ServerAddress,
        offered_codecs: Vec<Symbol>,
        allow_loopback: bool,
    ) -> Result<(Arc<dyn EvalSite>, Symbol)> {
        let kind = transport_kind(&address);
        let mut transport = open_connection_transport(&address, allow_loopback)?;
        let selected = if matches!(
            address,
            ServerAddress::Sse { .. } | ServerAddress::Wasm { .. }
        ) {
            select_default_codec(&offered_codecs)?
        } else {
            negotiate_codec(cx, transport.as_mut(), &offered_codecs)?
        };
        let site: Arc<dyn EvalSite> = Arc::new(Self {
            kind,
            address,
            codecs: offered_codecs,
            transport: Mutex::new(transport),
        });
        Ok((site, selected))
    }
}

impl EvalSite for TransportEvalSite {
    fn site_kind(&self) -> &'static str {
        self.kind
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: crate::ServerFrame) -> Result<crate::ServerFrame> {
        self.answer_with_timeout(cx, frame, None)
    }

    fn answer_with_timeout(
        &self,
        cx: &mut Cx,
        frame: crate::ServerFrame,
        timeout: Option<Duration>,
    ) -> Result<crate::ServerFrame> {
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| Error::HostError("transport eval site mutex poisoned".to_owned()))?;
        transport.send_frame(cx, frame)?;
        transport
            .recv_frame(cx, timeout)?
            .ok_or_else(|| match timeout {
                Some(timeout) => Error::Eval(format!(
                    "request timed out after {}",
                    format_duration(timeout)
                )),
                None => Error::HostError("transport did not return a frame".to_owned()),
            })
    }

    #[cfg(feature = "server-net-http")]
    fn stream(
        &self,
        cx: &mut Cx,
        frame: crate::ServerFrame,
        sink: &mut dyn crate::StreamSink,
    ) -> Result<()> {
        if let ServerAddress::Sse { .. } = &self.address {
            return sse_stream_request(cx, &self.address, frame, sink);
        }
        let reply = self.answer(cx, frame)?;
        sink.chunk(cx, reply)?;
        sink.end(cx)
    }

    fn close_connection(&self, cx: &mut Cx) -> Result<()> {
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| Error::HostError("transport eval site mutex poisoned".to_owned()))?;
        transport.close(cx)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
