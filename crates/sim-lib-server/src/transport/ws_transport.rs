use super::port_io::{PortTcpListener as TcpListener, PortTcpStream as TcpStream};
use std::{io::ErrorKind, net::Shutdown, sync::Arc, time::Duration};

use sim_kernel::{Cx, Error, Result, Symbol};

use crate::{
    EvalSite, ServerAddress, ServerFrame, ServerRuntime,
    http::{
        HttpRequest, HttpResponse, ParsedUrl, WsMessage, base64_encode, format_url, header_value,
        parse_url, read_request, read_response, read_ws_message, websocket_accept_value,
        write_request, write_response, write_ws_binary, write_ws_close,
    },
};

use super::{
    ConnectionTransport, SERVER_CONNECTION_IO_TIMEOUT_MS, ServerTransport, WS_TRANSPORT_PATH,
    answer_or_negotiate, decode_transport_frame, encode_transport_frame, error_frame_from_error,
    io_to_host, is_timeout, update_negotiated_codec_from_reply,
};

pub struct WsServerTransport {
    address: ServerAddress,
    listener: TcpListener,
    path: String,
}

impl WsServerTransport {
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Ws { url } = &address else {
            return Err(Error::Eval("ws transport requires a ws address".to_owned()));
        };
        let parsed = parse_url(url, "ws", WS_TRANSPORT_PATH)?;
        let listener =
            TcpListener::bind((parsed.host.as_str(), parsed.port)).map_err(io_to_host)?;
        listener.set_nonblocking(true).map_err(io_to_host)?;
        let local_addr = listener.local_addr().map_err(io_to_host)?;
        let address = ServerAddress::Ws {
            url: format_url(&ParsedUrl {
                port: local_addr.port(),
                ..parsed.clone()
            }),
        };
        Ok(Self {
            address,
            listener,
            path: parsed.path,
        })
    }
}

impl ServerTransport for WsServerTransport {
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
                Ok(Some(Box::new(WsServerConnectionTransport::new(
                    stream,
                    self.path.clone(),
                ))))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(io_to_host(error)),
        }
    }
}

pub struct WsConnectionTransport {
    stream: TcpStream,
}

impl WsConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Ws { url } = address else {
            return Err(Error::Eval("ws connect requires a ws address".to_owned()));
        };
        let parsed = parse_url(url, "ws", WS_TRANSPORT_PATH)?;
        let mut stream =
            TcpStream::connect((parsed.host.as_str(), parsed.port)).map_err(io_to_host)?;
        stream.set_nodelay(true).map_err(io_to_host)?;
        let client_key = base64_encode(b"sim-say-websocket");
        write_request(
            &mut stream,
            &HttpRequest {
                method: "GET".to_owned(),
                path: parsed.path,
                headers: vec![
                    ("Host".to_owned(), "sim-server".to_owned()),
                    ("Upgrade".to_owned(), "websocket".to_owned()),
                    ("Connection".to_owned(), "Upgrade".to_owned()),
                    ("Sec-WebSocket-Version".to_owned(), "13".to_owned()),
                    ("Sec-WebSocket-Key".to_owned(), client_key.clone()),
                ],
                body: Vec::new(),
            },
        )?;
        let response = read_response(&mut stream)?;
        if response.status != 101 {
            return Err(Error::Eval(format!("websocket status {}", response.status)));
        }
        let Some(accept) = header_value(&response.headers, "Sec-WebSocket-Accept") else {
            return Err(Error::HostError(
                "websocket handshake missing sec-websocket-accept".to_owned(),
            ));
        };
        if accept != websocket_accept_value(&client_key) {
            return Err(Error::HostError(
                "websocket handshake returned the wrong accept value".to_owned(),
            ));
        }
        Ok(Self { stream })
    }
}

impl ConnectionTransport for WsConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        write_ws_binary(&mut self.stream, &encode_transport_frame(&frame)?, true)
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        self.stream.set_read_timeout(timeout).map_err(io_to_host)?;
        match read_ws_message(&mut self.stream)? {
            Some(WsMessage::Binary(payload)) => decode_transport_frame(&payload).map(Some),
            Some(WsMessage::Close) => Ok(None),
            None => Ok(None),
        }
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let _ = write_ws_close(&mut self.stream, true);
        let _ = self.stream.shutdown(Shutdown::Both);
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct WsServerConnectionTransport {
    stream: TcpStream,
    path: String,
}

impl WsServerConnectionTransport {
    fn new(stream: TcpStream, path: String) -> Self {
        Self { stream, path }
    }

    fn serve(&mut self, runtime: &Arc<ServerRuntime>, site: &Arc<dyn EvalSite>) -> Result<()> {
        let handshake = match read_request(&mut self.stream)? {
            Some(request) => request,
            None => return Ok(()),
        };
        if handshake.method != "GET" {
            super::http_transport::write_http_error(&mut self.stream, 405, "method not allowed")?;
            return Ok(());
        }
        if handshake.path != self.path {
            super::http_transport::write_http_error(&mut self.stream, 404, "not found")?;
            return Ok(());
        }
        let Some(key) = header_value(&handshake.headers, "Sec-WebSocket-Key") else {
            super::http_transport::write_http_error(
                &mut self.stream,
                400,
                "missing sec-websocket-key",
            )?;
            return Ok(());
        };
        write_response(
            &mut self.stream,
            &HttpResponse {
                status: 101,
                headers: vec![
                    ("Upgrade".to_owned(), "websocket".to_owned()),
                    ("Connection".to_owned(), "Upgrade".to_owned()),
                    (
                        "Sec-WebSocket-Accept".to_owned(),
                        websocket_accept_value(key),
                    ),
                ],
                body: Vec::new(),
            },
        )?;

        let session_id = runtime.open_session(
            Symbol::qualified("codec", "binary"),
            runtime.session_isolation().clone(),
        )?;
        loop {
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
            self.stream
                .set_read_timeout(Some(Duration::from_millis(SERVER_CONNECTION_IO_TIMEOUT_MS)))
                .map_err(io_to_host)?;
            let message = match read_ws_message(&mut self.stream) {
                Ok(message) => message,
                Err(error) if is_timeout(&error) => continue,
                Err(error) => {
                    let _ = runtime.close_session(session_id);
                    return Err(error);
                }
            };
            let Some(message) = message else {
                let _ = runtime.close_session(session_id);
                return Ok(());
            };
            let WsMessage::Binary(payload) = message else {
                let _ = write_ws_close(&mut self.stream, false);
                let _ = runtime.close_session(session_id);
                return Ok(());
            };
            let frame = decode_transport_frame(&payload)?;
            runtime.note_message_received();
            let reply = match runtime.with_cx(|cx| answer_or_negotiate(cx, site, frame.clone())) {
                Ok(reply) => {
                    update_negotiated_codec_from_reply(runtime, session_id, &frame, &reply)?;
                    reply
                }
                Err(error) => runtime.with_cx(|cx| error_frame_from_error(cx, &frame, &error))?,
            };
            write_ws_binary(&mut self.stream, &encode_transport_frame(&reply)?, false)?;
            runtime.note_message_sent();
        }
    }
}

impl ConnectionTransport for WsServerConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, _frame: ServerFrame) -> Result<()> {
        Err(Error::Eval(
            "ws server connection transport is receive-only".to_owned(),
        ))
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        Err(Error::Eval(
            "ws server connection transport does not expose raw frames".to_owned(),
        ))
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let _ = write_ws_close(&mut self.stream, false);
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
