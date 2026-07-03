use std::{
    io::{BufReader, ErrorKind, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

use sim_kernel::{Cx, Error, Result, Symbol};

use crate::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, ServerRuntime, StreamSink,
    http::{
        HttpRequest, HttpResponse, ParsedUrl, base64_decode, base64_encode, format_url,
        header_value, parse_url, read_request, read_response, read_sse_event, write_request,
        write_response,
    },
};

use super::{
    ConnectionTransport, HTTP_TRANSPORT_PATH, SSE_TRANSPORT_PATH, ServerTransport,
    decode_transport_frame, encode_transport_frame, io_to_host, update_negotiated_codec_from_reply,
};

pub struct SseServerTransport {
    address: ServerAddress,
    listener: TcpListener,
    path: String,
}

impl SseServerTransport {
    pub fn bind(address: ServerAddress) -> Result<Self> {
        let ServerAddress::Sse { url } = &address else {
            return Err(Error::Eval(
                "sse transport requires an sse address".to_owned(),
            ));
        };
        let parsed = parse_url(url, "http", SSE_TRANSPORT_PATH)?;
        let listener =
            TcpListener::bind((parsed.host.as_str(), parsed.port)).map_err(io_to_host)?;
        listener.set_nonblocking(true).map_err(io_to_host)?;
        let local_addr = listener.local_addr().map_err(io_to_host)?;
        let address = ServerAddress::Sse {
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

impl ServerTransport for SseServerTransport {
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
                Ok(Some(Box::new(SseServerConnectionTransport::new(
                    stream,
                    self.path.clone(),
                ))))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(io_to_host(error)),
        }
    }
}

pub struct SseConnectionTransport {
    address: ServerAddress,
    inner: Option<super::HttpConnectionTransport>,
}

impl SseConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Sse { url } = address else {
            return Err(Error::Eval(
                "sse connect requires an sse address".to_owned(),
            ));
        };
        let parsed = parse_url(url, "http", SSE_TRANSPORT_PATH)?;
        Ok(Self {
            address: ServerAddress::Http {
                url: format_url(&ParsedUrl {
                    path: HTTP_TRANSPORT_PATH.to_owned(),
                    ..parsed
                }),
            },
            inner: None,
        })
    }

    fn inner_mut(&mut self) -> Result<&mut super::HttpConnectionTransport> {
        if self.inner.is_none() {
            self.inner = Some(super::HttpConnectionTransport::connect(&self.address)?);
        }
        self.inner.as_mut().ok_or_else(|| {
            Error::HostError("sse http fallback transport was not initialized".to_owned())
        })
    }
}

impl ConnectionTransport for SseConnectionTransport {
    fn send_frame(&mut self, cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        self.inner_mut()?.send_frame(cx, frame)
    }

    fn recv_frame(
        &mut self,
        cx: &mut Cx,
        timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        self.inner_mut()?.recv_frame(cx, timeout)
    }

    fn close(&mut self, cx: &mut Cx) -> Result<()> {
        if let Some(inner) = &mut self.inner {
            inner.close(cx)?;
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct SseServerConnectionTransport {
    stream: TcpStream,
    path: String,
}

impl SseServerConnectionTransport {
    fn new(stream: TcpStream, path: String) -> Self {
        Self { stream, path }
    }

    fn serve(&mut self, runtime: &Arc<ServerRuntime>, site: &Arc<dyn EvalSite>) -> Result<()> {
        let session_id = runtime.open_session(
            Symbol::qualified("codec", "binary"),
            runtime.session_isolation().clone(),
        )?;
        let request = match read_request(&mut self.stream)? {
            Some(request) => request,
            None => {
                let _ = runtime.close_session(session_id);
                return Ok(());
            }
        };
        if request.method != "GET" {
            super::http_transport::write_http_error(&mut self.stream, 405, "method not allowed")?;
            let _ = runtime.close_session(session_id);
            return Ok(());
        }
        if request.path != self.path {
            super::http_transport::write_http_error(&mut self.stream, 404, "not found")?;
            let _ = runtime.close_session(session_id);
            return Ok(());
        }
        let Some(frame_header) = header_value(&request.headers, "X-Sim-Frame") else {
            super::http_transport::write_http_error(
                &mut self.stream,
                400,
                "missing x-sim-frame header",
            )?;
            let _ = runtime.close_session(session_id);
            return Ok(());
        };
        let frame = decode_transport_frame(&base64_decode(frame_header)?)?;
        runtime.note_message_received();
        write_response(
            &mut self.stream,
            &HttpResponse {
                status: 200,
                headers: vec![
                    ("Content-Type".to_owned(), "text/event-stream".to_owned()),
                    ("Cache-Control".to_owned(), "no-cache".to_owned()),
                ],
                body: Vec::new(),
            },
        )?;
        let mut sink = SseStreamSink {
            stream: &mut self.stream,
            sent_end: false,
            runtime,
        };
        update_negotiated_codec_from_reply(runtime, session_id, &frame, &frame)?;
        let outcome = runtime.with_cx(|cx| site.stream(cx, frame, &mut sink));
        let _ = sink.end_without_cx();
        let _ = runtime.close_session(session_id);
        outcome
    }
}

impl ConnectionTransport for SseServerConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, _frame: ServerFrame) -> Result<()> {
        Err(Error::Eval(
            "sse server connection transport is receive-only".to_owned(),
        ))
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<Duration>,
    ) -> Result<Option<ServerFrame>> {
        Err(Error::Eval(
            "sse server connection transport does not expose raw frames".to_owned(),
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

struct SseStreamSink<'a> {
    stream: &'a mut TcpStream,
    sent_end: bool,
    runtime: &'a Arc<ServerRuntime>,
}

impl SseStreamSink<'_> {
    fn write_event(&mut self, event: &str, data: &str) -> Result<()> {
        write!(self.stream, "event: {event}\r\ndata: {data}\r\n\r\n").map_err(io_to_host)?;
        self.stream.flush().map_err(io_to_host)
    }

    fn end_without_cx(&mut self) -> Result<()> {
        if self.sent_end {
            return Ok(());
        }
        self.write_event("end", "")?;
        self.sent_end = true;
        Ok(())
    }
}

fn sse_event_name_for_frame(kind: &FrameKind) -> &'static str {
    match kind {
        FrameKind::StreamStart => "stream-start",
        FrameKind::StreamChunk => "stream-chunk",
        FrameKind::StreamEnd => "stream-end",
        _ => "chunk",
    }
}

impl StreamSink for SseStreamSink<'_> {
    fn chunk(&mut self, _cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        let event = sse_event_name_for_frame(&frame.kind);
        let payload = encode_transport_frame(&frame)?;
        self.write_event(event, &base64_encode(&payload))?;
        self.runtime.note_message_sent();
        Ok(())
    }

    fn end(&mut self, _cx: &mut Cx) -> Result<()> {
        self.end_without_cx()
    }
}

pub(super) fn sse_stream_request(
    cx: &mut Cx,
    address: &ServerAddress,
    frame: ServerFrame,
    sink: &mut dyn StreamSink,
) -> Result<()> {
    let ServerAddress::Sse { url } = address else {
        return Err(Error::Eval("sse stream requires an sse address".to_owned()));
    };
    let parsed = parse_url(url, "http", SSE_TRANSPORT_PATH)?;
    let mut stream = TcpStream::connect((parsed.host.as_str(), parsed.port)).map_err(io_to_host)?;
    stream.set_nodelay(true).map_err(io_to_host)?;
    let request_frame = base64_encode(&encode_transport_frame(&frame)?);
    write_request(
        &mut stream,
        &HttpRequest {
            method: "GET".to_owned(),
            path: parsed.path,
            headers: vec![
                ("Host".to_owned(), "sim-server".to_owned()),
                ("Accept".to_owned(), "text/event-stream".to_owned()),
                ("X-Sim-Frame".to_owned(), request_frame),
            ],
            body: Vec::new(),
        },
    )?;
    let response = read_response(&mut stream)?;
    if response.status != 200 {
        return Err(Error::Eval(format!("sse status {}", response.status)));
    }
    let mut reader = BufReader::new(stream);
    while let Some((event, data)) = read_sse_event(&mut reader)? {
        match event.as_str() {
            "chunk" | "stream-start" | "stream-chunk" | "stream-end" => {
                let payload = base64_decode(&data)?;
                let frame = decode_transport_frame(&payload)?;
                sink.chunk(cx, frame)?;
            }
            "end" => {
                sink.end(cx)?;
                return Ok(());
            }
            _ => {}
        }
    }
    sink.end(cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_event_names_preserve_stream_frame_kinds() {
        assert_eq!(
            sse_event_name_for_frame(&FrameKind::StreamStart),
            "stream-start"
        );
        assert_eq!(
            sse_event_name_for_frame(&FrameKind::StreamChunk),
            "stream-chunk"
        );
        assert_eq!(
            sse_event_name_for_frame(&FrameKind::StreamEnd),
            "stream-end"
        );
        assert_eq!(sse_event_name_for_frame(&FrameKind::Response), "chunk");
    }
}
