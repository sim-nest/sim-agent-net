use super::port_io::{PortTcpListener as TcpListener, PortTcpStream as TcpStream};
use std::{io::ErrorKind, net::Shutdown, sync::Arc, time::Duration};

use sim_kernel::{Cx, Error, Result, Symbol};

use crate::{
    EvalSite, FrameEnvelope, FrameKind, ServerAddress, ServerFrame, ServerRuntime,
    http::{
        HttpRequest, HttpResponse, ParsedUrl, format_url, header_value, parse_url, read_request,
        read_response, write_request, write_response,
    },
};

use super::{
    ConnectionTransport, HTTP_TRANSPORT_PATH, SERVER_CONNECTION_IO_TIMEOUT_MS, ServerTransport,
    answer_or_negotiate, decode_transport_frame, encode_transport_frame, error_frame_from_error,
    io_to_host, is_timeout, update_negotiated_codec_from_reply,
};

pub struct HttpServerTransport {
    address: ServerAddress,
    listener: TcpListener,
    path: String,
}

impl HttpServerTransport {
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Http { url } = &address else {
            return Err(Error::Eval(
                "http transport requires an http address".to_owned(),
            ));
        };
        let parsed = parse_url(url, "http", HTTP_TRANSPORT_PATH)?;
        let listener =
            TcpListener::bind((parsed.host.as_str(), parsed.port)).map_err(io_to_host)?;
        listener.set_nonblocking(true).map_err(io_to_host)?;
        let local_addr = listener.local_addr().map_err(io_to_host)?;
        let address = ServerAddress::Http {
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

impl ServerTransport for HttpServerTransport {
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
                Ok(Some(Box::new(HttpServerConnectionTransport::new(
                    stream,
                    self.path.clone(),
                ))))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(io_to_host(error)),
        }
    }
}

pub struct HttpConnectionTransport {
    stream: TcpStream,
    path: String,
}

impl HttpConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Http { url } = address else {
            return Err(Error::Eval(
                "http connect requires an http address".to_owned(),
            ));
        };
        let parsed = parse_url(url, "http", HTTP_TRANSPORT_PATH)?;
        let stream = TcpStream::connect((parsed.host.as_str(), parsed.port)).map_err(io_to_host)?;
        stream.set_nodelay(true).map_err(io_to_host)?;
        Ok(Self {
            stream,
            path: parsed.path,
        })
    }
}

impl ConnectionTransport for HttpConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        let body = encode_transport_frame(&frame)?;
        write_request(
            &mut self.stream,
            &HttpRequest {
                method: "POST".to_owned(),
                path: self.path.clone(),
                headers: vec![
                    ("Host".to_owned(), "sim-server".to_owned()),
                    (
                        "Content-Type".to_owned(),
                        "application/sim-frame".to_owned(),
                    ),
                    ("Content-Length".to_owned(), body.len().to_string()),
                ],
                body,
            },
        )
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        self.stream.set_read_timeout(timeout).map_err(io_to_host)?;
        let response = read_response(&mut self.stream)?;
        if response.status != 200 {
            return Err(Error::Eval(format!("http status {}", response.status)));
        }
        decode_transport_frame(&response.body).map(Some)
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let _ = self.stream.shutdown(Shutdown::Both);
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct HttpServerConnectionTransport {
    stream: TcpStream,
    path: String,
}

impl HttpServerConnectionTransport {
    fn new(stream: TcpStream, path: String) -> Self {
        Self { stream, path }
    }

    fn serve(&mut self, runtime: &Arc<ServerRuntime>, site: &Arc<dyn EvalSite>) -> Result<()> {
        let session_id = runtime.open_session(
            Symbol::qualified("codec", "binary"),
            runtime.session_isolation().clone(),
        )?;
        loop {
            if runtime.is_stopping() {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
            let request = match self.recv_request_for_serve() {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(error) => {
                    let _ = runtime.close_session(session_id);
                    return Err(error);
                }
            };
            let Some(request) = request else {
                let _ = runtime.close_session(session_id);
                return Ok(());
            };
            if request.method != "POST" {
                write_http_error(&mut self.stream, 405, "method not allowed")?;
                continue;
            }
            if request.path != self.path {
                write_http_error(&mut self.stream, 404, "not found")?;
                continue;
            }
            let Some(content_type) = header_value(&request.headers, "Content-Type") else {
                write_http_error(&mut self.stream, 400, "missing content-type")?;
                continue;
            };
            if content_type != "application/sim-frame" {
                write_http_error(&mut self.stream, 400, "unexpected content-type")?;
                continue;
            }
            let frame = match decode_transport_frame(&request.body) {
                Ok(frame) => frame,
                Err(error) => {
                    // An undecodable frame body must not tear down the whole
                    // connection. Reply with a structured error frame, matching
                    // the eval-error path below, and keep serving. The session
                    // codec is binary (see open_session above), so encode the
                    // error under a binary fallback frame.
                    runtime.note_message_received();
                    let fallback = ServerFrame::new(
                        Symbol::qualified("codec", "binary"),
                        FrameKind::Error,
                        FrameEnvelope::default(),
                        Vec::new(),
                    );
                    let reply =
                        runtime.with_cx(|cx| error_frame_from_error(cx, &fallback, &error))?;
                    let body = encode_transport_frame(&reply)?;
                    write_response(
                        &mut self.stream,
                        &HttpResponse {
                            status: 200,
                            headers: vec![(
                                "Content-Type".to_owned(),
                                "application/sim-frame".to_owned(),
                            )],
                            body,
                        },
                    )?;
                    runtime.note_message_sent();
                    continue;
                }
            };
            runtime.note_message_received();
            let reply = match runtime.with_cx(|cx| answer_or_negotiate(cx, site, frame.clone())) {
                Ok(reply) => {
                    update_negotiated_codec_from_reply(runtime, session_id, &frame, &reply)?;
                    reply
                }
                Err(error) => runtime.with_cx(|cx| error_frame_from_error(cx, &frame, &error))?,
            };
            let body = encode_transport_frame(&reply)?;
            write_response(
                &mut self.stream,
                &HttpResponse {
                    status: 200,
                    headers: vec![(
                        "Content-Type".to_owned(),
                        "application/sim-frame".to_owned(),
                    )],
                    body,
                },
            )?;
            runtime.note_message_sent();
        }
    }

    fn recv_request_for_serve(&mut self) -> Result<Option<Option<HttpRequest>>> {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(SERVER_CONNECTION_IO_TIMEOUT_MS)))
            .map_err(io_to_host)?;
        match read_request(&mut self.stream) {
            Ok(request) => Ok(Some(request)),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl ConnectionTransport for HttpServerConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, _frame: ServerFrame) -> Result<()> {
        Err(Error::Eval(
            "http server connection transport is receive-only".to_owned(),
        ))
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        Err(Error::Eval(
            "http server connection transport does not expose raw frames".to_owned(),
        ))
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

pub(super) fn write_http_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
    write_response(
        stream,
        &HttpResponse {
            status,
            headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
            body: message.as_bytes().to_vec(),
        },
    )
}
