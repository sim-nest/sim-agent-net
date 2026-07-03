use std::{
    fs,
    io::ErrorKind,
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
    sync::Arc,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::{
    fs::FileTypeExt,
    net::{UnixListener, UnixStream},
};

use sim_kernel::{Cx, Error, Result, Symbol};

use crate::{EvalSite, FrameKind, ServerAddress, ServerFrame, ServerRuntime};

use super::{
    ConnectionTransport, SERVER_CONNECTION_IO_TIMEOUT_MS, ServerTransport, answer_or_negotiate,
    error_frame_from_error, io_to_host, is_timeout, read_frame_from,
    update_negotiated_codec_from_reply, write_frame_to,
};

/// TCP listener transport for server-frame connections.
pub struct TcpServerTransport {
    address: ServerAddress,
    listener: TcpListener,
}

impl TcpServerTransport {
    /// Binds a TCP listener to `address`.
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Tcp { host, port } = &address else {
            return Err(Error::Eval(
                "tcp transport requires a tcp address".to_owned(),
            ));
        };
        let listener = TcpListener::bind((host.as_str(), *port)).map_err(io_to_host)?;
        listener.set_nonblocking(true).map_err(io_to_host)?;
        let local_addr = listener.local_addr().map_err(io_to_host)?;
        Ok(Self {
            address: ServerAddress::Tcp {
                host: host.clone(),
                port: local_addr.port(),
            },
            listener,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    /// Returns the bound local port.
    pub fn local_port(&self) -> Result<u16> {
        Ok(self.listener.local_addr().map_err(io_to_host)?.port())
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
        match self.listener.accept() {
            Ok((stream, _peer)) => {
                stream.set_nodelay(true).map_err(io_to_host)?;
                Ok(Some(Box::new(TcpConnectionTransport::server_side(stream))))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(io_to_host(error)),
        }
    }
}

pub struct TcpConnectionTransport {
    stream: TcpStream,
}

impl TcpConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Tcp { host, port } = address else {
            return Err(Error::Eval("tcp connect requires a tcp address".to_owned()));
        };
        let stream = TcpStream::connect((host.as_str(), *port)).map_err(io_to_host)?;
        stream.set_nodelay(true).map_err(io_to_host)?;
        Ok(Self { stream })
    }

    fn server_side(stream: TcpStream) -> Self {
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
            .map_err(io_to_host)?;
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
        self.stream.set_read_timeout(timeout).map_err(io_to_host)?;
        match read_frame_from(&mut self.stream) {
            Ok(frame) => Ok(frame),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let _ = self.stream.shutdown(Shutdown::Both);
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
    listener: UnixListener,
}

#[cfg(unix)]
impl UnixServerTransport {
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Unix { path } = &address else {
            return Err(Error::Eval(
                "unix transport requires a unix address".to_owned(),
            ));
        };
        remove_stale_unix_socket(path)?;
        let listener = UnixListener::bind(path).map_err(io_to_host)?;
        listener.set_nonblocking(true).map_err(io_to_host)?;
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
        let ServerAddress::Unix { path } = &self.address else {
            return Ok(());
        };
        remove_bound_unix_socket(path)
    }

    fn accept_timeout(
        &self,
        _cx: &mut Cx,
        _timeout: Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>> {
        match self.listener.accept() {
            Ok((stream, _peer)) => Ok(Some(Box::new(UnixConnectionTransport::server_side(stream)))),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(io_to_host(error)),
        }
    }
}

#[cfg(unix)]
pub struct UnixConnectionTransport {
    stream: UnixStream,
}

#[cfg(unix)]
impl UnixConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Unix { path } = address else {
            return Err(Error::Eval(
                "unix connect requires a unix address".to_owned(),
            ));
        };
        let stream = UnixStream::connect(path).map_err(io_to_host)?;
        Ok(Self { stream })
    }

    fn server_side(stream: UnixStream) -> Self {
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
            .map_err(io_to_host)?;
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
        self.stream.set_read_timeout(timeout).map_err(io_to_host)?;
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

#[cfg(unix)]
fn remove_stale_unix_socket(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(io_to_host)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_to_host(error)),
    }
}

#[cfg(unix)]
fn remove_bound_unix_socket(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(io_to_host)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_to_host(error)),
    }
}
