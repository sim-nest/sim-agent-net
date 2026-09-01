use std::{sync::Arc, time::Duration};

use sim_transport_ports::{Half, IpcAddress, IpcListener, Listener, SocketAddress, Stream};

use sim_kernel::{Cx, Error, Result, Symbol};

use crate::{EvalSite, FrameKind, ServerAddress, ServerFrame, ServerRuntime};

use super::{
    ConnectionTransport, SERVER_CONNECTION_IO_TIMEOUT_MS, ServerTransport, answer_or_negotiate,
    bound_transport_services, error_frame_from_error, is_timeout, read_frame_from,
    update_negotiated_codec_from_reply, write_frame_to,
};

/// TCP listener transport for server-frame connections.
pub struct TcpServerTransport {
    address: ServerAddress,
    listener: Box<dyn Listener>,
}

impl TcpServerTransport {
    /// Binds a TCP listener to `address`.
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Tcp { host, port } = &address else {
            return Err(Error::Eval(
                "tcp transport requires a tcp address".to_owned(),
            ));
        };
        let ports = bound_transport_services().map_err(port_error)?;
        let resolved = ports.dns.resolve(host, *port).map_err(port_error)?;
        let target = resolved
            .first()
            .ok_or_else(|| Error::HostError("DNS returned no addresses".to_owned()))?;
        let listener = ports.sockets.listen_tcp(target).map_err(port_error)?;
        let local_addr = listener.local_address().map_err(port_error)?;
        let SocketAddress::Ip {
            port: local_port, ..
        } = local_addr;
        Ok(Self {
            address: ServerAddress::Tcp {
                host: host.clone(),
                port: local_port,
            },
            listener,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// Returns the bound local port.
    pub fn local_port(&self) -> Result<u16> {
        let SocketAddress::Ip { port, .. } = self.listener.local_address().map_err(port_error)?;
        Ok(port)
    }
}

impl ServerTransport for TcpServerTransport {
    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn accept(&self, cx: &mut Cx) -> Result<Box<dyn ConnectionTransport>> {
        loop {
            if let Some(connection) = self.accept_timeout(cx, Duration::from_millis(25))? {
                return Ok(connection);
            }
        }
    }

    fn shutdown(&self, _cx: &mut Cx) -> Result<()> {
        Ok(())
    }

    fn accept_timeout(
        &self,
        _cx: &mut Cx,
        _timeout: Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>> {
        self.listener
            .accept()
            .map(|stream| {
                stream.map(|stream| {
                    Box::new(TcpConnectionTransport::server_side(stream))
                        as Box<dyn ConnectionTransport>
                })
            })
            .map_err(port_error)
    }
}

pub struct TcpConnectionTransport {
    stream: Box<dyn Stream>,
}

impl TcpConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Tcp { host, port } = address else {
            return Err(Error::Eval("tcp connect requires a tcp address".to_owned()));
        };
        let ports = bound_transport_services().map_err(port_error)?;
        let resolved = ports.dns.resolve(host, *port).map_err(port_error)?;
        let target = resolved
            .first()
            .ok_or_else(|| Error::HostError("DNS returned no addresses".to_owned()))?;
        let stream = ports.sockets.connect_tcp(target).map_err(port_error)?;
        Ok(Self { stream })
    }

    fn server_side(stream: Box<dyn Stream>) -> Self {
        Self { stream }
    }

    fn serve(&mut self, runtime: &Arc<ServerRuntime>, site: &Arc<dyn EvalSite>) -> Result<()> {
        let session_id = runtime.open_session(
            Symbol::qualified("codec", "binary"),
            runtime.session_isolation().clone(),
        )?;
        let mut inflight = 0usize;
        loop {
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }

            let frame = match self.recv_frame_for_serve() {
                Ok(Some(frame)) => frame,
                Ok(None) => continue,
                Err(error) => {
                    let _ = runtime.close_session(session_id);
                    return Err(error);
                }
            };
            let Some(frame) = frame else {
                let _ = runtime.close_session(session_id);
                return Ok(());
            };
            runtime.note_message_received();
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
            if matches!(frame.kind, FrameKind::Request | FrameKind::Notify)
                && inflight >= runtime.max_inflight()
            {
                let reply = runtime.with_cx(|cx| {
                    error_frame_from_error(
                        cx,
                        &frame,
                        &Error::Eval(format!(
                            "connection max-inflight {} exceeded",
                            runtime.max_inflight()
                        )),
                    )
                })?;
                write_frame_to(&mut self.stream, &reply)?;
                runtime.note_message_sent();
                continue;
            }
            if matches!(frame.kind, FrameKind::Request | FrameKind::Notify) {
                inflight = inflight.saturating_add(1);
            }
            let reply = match runtime.with_cx(|cx| answer_or_negotiate(cx, site, frame.clone())) {
                Ok(reply) => {
                    update_negotiated_codec_from_reply(runtime, session_id, &frame, &reply)?;
                    reply
                }
                Err(error) => runtime.with_cx(|cx| error_frame_from_error(cx, &frame, &error))?,
            };
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
            write_frame_to(&mut self.stream, &reply)?;
            runtime.note_message_sent();
            if matches!(frame.kind, FrameKind::Request | FrameKind::Notify) {
                inflight = inflight.saturating_sub(1);
            }
        }
    }

    fn recv_frame_for_serve(&mut self) -> Result<Option<Option<ServerFrame>>> {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(SERVER_CONNECTION_IO_TIMEOUT_MS)))
            .map_err(port_error)?;
        match read_frame_from(&mut self.stream) {
            Ok(frame) => Ok(Some(frame)),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl ConnectionTransport for TcpConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        write_frame_to(&mut self.stream, &frame)
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        self.stream.set_read_timeout(timeout).map_err(port_error)?;
        match read_frame_from(&mut self.stream) {
            Ok(frame) => Ok(frame),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let _ = self.stream.shutdown(Half::Both);
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn serve_connection(
        &mut self,
        runtime: &Arc<ServerRuntime>,
        site: &Arc<dyn EvalSite>,
    ) -> Result<()> {
        self.serve(runtime, site)
    }
}

#[cfg(unix)]
pub struct UnixServerTransport {
    address: ServerAddress,
    listener: Box<dyn IpcListener>,
}

#[cfg(unix)]
impl UnixServerTransport {
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Unix { path } = &address else {
            return Err(Error::Eval(
                "unix transport requires a unix address".to_owned(),
            ));
        };
        let listener = bound_transport_services()
            .map_err(port_error)?
            .ipc
            .ok_or_else(|| Error::HostError("local IPC service is unavailable".to_owned()))?
            .listen(&IpcAddress::UnixPath(path.clone()))
            .map_err(port_error)?;
        Ok(Self { address, listener })
    }
}

#[cfg(unix)]
impl ServerTransport for UnixServerTransport {
    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn accept(&self, cx: &mut Cx) -> Result<Box<dyn ConnectionTransport>> {
        loop {
            if let Some(connection) = self.accept_timeout(cx, Duration::from_millis(25))? {
                return Ok(connection);
            }
        }
    }

    fn shutdown(&self, _cx: &mut Cx) -> Result<()> {
        self.listener.close().map_err(port_error)
    }

    fn accept_timeout(
        &self,
        _cx: &mut Cx,
        _timeout: Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>> {
        self.listener
            .accept()
            .map(|stream| {
                stream.map(|stream| {
                    Box::new(UnixConnectionTransport::server_side(stream))
                        as Box<dyn ConnectionTransport>
                })
            })
            .map_err(port_error)
    }
}

#[cfg(unix)]
pub struct UnixConnectionTransport {
    stream: Box<dyn Stream>,
}

#[cfg(unix)]
impl UnixConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Unix { path } = address else {
            return Err(Error::Eval(
                "unix connect requires a unix address".to_owned(),
            ));
        };
        let stream = bound_transport_services()
            .map_err(port_error)?
            .ipc
            .ok_or_else(|| Error::HostError("local IPC service is unavailable".to_owned()))?
            .connect(&IpcAddress::UnixPath(path.clone()))
            .map_err(port_error)?;
        Ok(Self { stream })
    }

    fn server_side(stream: Box<dyn Stream>) -> Self {
        Self { stream }
    }

    fn serve(&mut self, runtime: &Arc<ServerRuntime>, site: &Arc<dyn EvalSite>) -> Result<()> {
        let session_id = runtime.open_session(
            Symbol::qualified("codec", "binary"),
            runtime.session_isolation().clone(),
        )?;
        let mut inflight = 0usize;
        loop {
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }

            let frame = match self.recv_frame_for_serve() {
                Ok(Some(frame)) => frame,
                Ok(None) => continue,
                Err(error) => {
                    let _ = runtime.close_session(session_id);
                    return Err(error);
                }
            };
            let Some(frame) = frame else {
                let _ = runtime.close_session(session_id);
                return Ok(());
            };
            runtime.note_message_received();
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
            if matches!(frame.kind, FrameKind::Request | FrameKind::Notify)
                && inflight >= runtime.max_inflight()
            {
                let reply = runtime.with_cx(|cx| {
                    error_frame_from_error(
                        cx,
                        &frame,
                        &Error::Eval(format!(
                            "connection max-inflight {} exceeded",
                            runtime.max_inflight()
                        )),
                    )
                })?;
                write_frame_to(&mut self.stream, &reply)?;
                runtime.note_message_sent();
                continue;
            }
            if matches!(frame.kind, FrameKind::Request | FrameKind::Notify) {
                inflight = inflight.saturating_add(1);
            }
            let reply = match runtime.with_cx(|cx| answer_or_negotiate(cx, site, frame.clone())) {
                Ok(reply) => {
                    update_negotiated_codec_from_reply(runtime, session_id, &frame, &reply)?;
                    reply
                }
                Err(error) => runtime.with_cx(|cx| error_frame_from_error(cx, &frame, &error))?,
            };
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
            write_frame_to(&mut self.stream, &reply)?;
            runtime.note_message_sent();
            if matches!(frame.kind, FrameKind::Request | FrameKind::Notify) {
                inflight = inflight.saturating_sub(1);
            }
        }
    }

    fn recv_frame_for_serve(&mut self) -> Result<Option<Option<ServerFrame>>> {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(SERVER_CONNECTION_IO_TIMEOUT_MS)))
            .map_err(port_error)?;
        match read_frame_from(&mut self.stream) {
            Ok(frame) => Ok(Some(frame)),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(unix)]
impl ConnectionTransport for UnixConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        write_frame_to(&mut self.stream, &frame)
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        self.stream.set_read_timeout(timeout).map_err(port_error)?;
        match read_frame_from(&mut self.stream) {
            Ok(frame) => Ok(frame),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn serve_connection(
        &mut self,
        runtime: &Arc<ServerRuntime>,
        site: &Arc<dyn EvalSite>,
    ) -> Result<()> {
        self.serve(runtime, site)
    }
}

fn port_error(error: sim_transport_ports::TransportError) -> Error {
    Error::HostError(error.to_string())
}
