//! Bounded streaming HTTP service seam over the platform transport ports.

use sim_cancel::{Cancellation, CancellationReason};
use sim_kernel::{Error, Result};
use std::{io, sync::Mutex, time::Duration};

// conformance: raw HTTP streaming is bounded, backpressured, and request-cancellable.

/// One ordered HTTP header. Names retain their received spelling and duplicates retain order.
pub type Header = (String, String);

/// Immutable request facts parsed once by the owning connection loop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestHead {
    /// Request method token.
    pub method: String,
    /// Origin-form or absolute request target.
    pub target: String,
    /// Ordered headers, including duplicates.
    pub headers: Vec<Header>,
    /// Peer address reported by the socket provider.
    pub peer: Option<String>,
    /// Local address reported by the socket provider.
    pub local: Option<String>,
}

/// Response facts emitted before the first body byte.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseHead {
    /// Numeric HTTP status.
    pub status: u16,
    /// Ordered response headers.
    pub headers: Vec<Header>,
}

/// Fixed memory and wire bounds for one raw request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BodyLimits {
    /// Maximum bytes accepted for the complete request body.
    pub max_request_bytes: usize,
    /// Maximum bytes admitted in one read or write chunk.
    pub max_chunk_bytes: usize,
}

impl BodyLimits {
    fn validate(self) -> Result<Self> {
        if self.max_request_bytes == 0 || self.max_chunk_bytes == 0 {
            return Err(Error::Eval("raw HTTP body limits must be non-zero".into()));
        }
        Ok(self)
    }
}

/// Whether a handler may finish a response with trailers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrailersPolicy {
    /// Reject every trailer.
    Deny,
    /// Permit bounded ordered trailers.
    Allow,
}

/// Cancellation and deadline owned by exactly one handler invocation.
#[derive(Clone, Debug)]
pub struct RequestScope {
    cancellation: Cancellation,
    deadline: Duration,
}

impl RequestScope {
    /// Creates an independent request scope beneath the caller/server lifetime.
    #[must_use]
    pub fn child(parent: &Cancellation, deadline: Duration) -> Self {
        Self {
            cancellation: parent.child(),
            deadline,
        }
    }
    /// Returns the request cancellation observer.
    #[must_use]
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
    /// Returns the host-clock-relative deadline budget.
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.deadline
    }
    /// Records that the injected platform clock reached the request deadline.
    pub fn cancel_timeout(&self) {
        self.cancel("request deadline reached");
    }
    /// Records EOF or another peer-side disconnect observed by the connection adapter.
    pub fn cancel_peer_drop(&self) {
        self.cancel("peer disconnected");
    }
    fn cancel(&self, reason: &'static str) {
        self.cancellation
            .cancel(CancellationReason::new(reason).expect("static reason is valid"));
    }
}

/// Pull-based bounded request body. A chunk is consumed before another can be requested.
pub trait BodyReader {
    /// Returns the next non-empty chunk, or `None` at the message boundary.
    fn next_chunk(&mut self, scope: &RequestScope) -> io::Result<Option<Vec<u8>>>;
}

/// Push-based response body with write completion as its backpressure acknowledgement.
pub trait ResponseWriter {
    /// Emits the response head exactly once.
    fn write_head(&mut self, head: ResponseHead, scope: &RequestScope) -> io::Result<()>;
    /// Emits one bounded chunk and returns only when the connection accepts it.
    fn write_chunk(&mut self, chunk: &[u8], scope: &RequestScope) -> io::Result<()>;
    /// Completes the body, subject to the server trailer policy.
    fn finish(&mut self, trailers: &[Header], scope: &RequestScope) -> io::Result<()>;
}

/// Raw connection after the shared accept loop and HTTP parser have produced a request head.
pub trait RawConnection {
    /// Borrows the parsed head and independent streaming halves together.
    fn parts(&mut self) -> (&RequestHead, &mut dyn BodyReader, &mut dyn ResponseWriter);
}

/// Application boundary for one raw HTTP request.
pub trait RawHandler: Send + Sync {
    /// Handles one request without owning a socket, parser, executor, or clock.
    fn handle(
        &self,
        head: &RequestHead,
        body: &mut dyn BodyReader,
        response: &mut dyn ResponseWriter,
        scope: &RequestScope,
    ) -> Result<()>;
}

/// Policy-bearing dispatcher used by the existing HTTP accept/parser loop.
pub struct RawHttpServer<H> {
    handler: H,
    limits: BodyLimits,
    trailers: TrailersPolicy,
    request_deadline: Duration,
    shutdown: Cancellation,
    active: Mutex<Vec<Cancellation>>,
}

impl<H: RawHandler> RawHttpServer<H> {
    /// Creates a raw dispatcher. It deliberately does not create a listener or runtime.
    pub fn new(
        handler: H,
        limits: BodyLimits,
        trailers: TrailersPolicy,
        request_deadline: Duration,
    ) -> Result<Self> {
        if request_deadline.is_zero() {
            return Err(Error::Eval(
                "raw HTTP request deadline must be non-zero".into(),
            ));
        }
        Ok(Self {
            handler,
            limits: limits.validate()?,
            trailers,
            request_deadline,
            shutdown: Cancellation::new(),
            active: Mutex::new(Vec::new()),
        })
    }
    /// Cancels current and future request children during server shutdown.
    pub fn shutdown(&self) {
        self.shutdown
            .cancel(CancellationReason::new("server shutdown").expect("static reason is valid"));
        for request in self
            .active
            .lock()
            .expect("active request mutex poisoned")
            .drain(..)
        {
            request.cancel(
                CancellationReason::new("server shutdown").expect("static reason is valid"),
            );
        }
    }
    /// Dispatches one already parsed connection through a fresh request scope.
    pub fn serve(&self, connection: &mut dyn RawConnection, caller: &Cancellation) -> Result<()> {
        let scope = RequestScope::child(caller, self.request_deadline);
        if self.shutdown.is_cancelled() {
            scope.cancel("server shutdown");
        }
        self.active
            .lock()
            .expect("active request mutex poisoned")
            .push(scope.cancellation.clone());
        let (head, body, response) = connection.parts();
        let head = head.clone();
        let mut body = LimitedBody {
            inner: body,
            limits: self.limits,
            received: 0,
        };
        let body: &mut dyn BodyReader = &mut body;
        let mut response = LimitedResponse {
            inner: response,
            max_chunk: self.limits.max_chunk_bytes,
            trailers: self.trailers,
        };
        let result = self.handler.handle(&head, body, &mut response, &scope);
        if result.is_err() {
            scope.cancel("handler failure");
        }
        scope.cancel("request complete");
        self.active
            .lock()
            .expect("active request mutex poisoned")
            .retain(|request| !request.is_cancelled());
        result
    }
}

struct LimitedBody<'a> {
    inner: &'a mut dyn BodyReader,
    limits: BodyLimits,
    received: usize,
}
impl BodyReader for LimitedBody<'_> {
    fn next_chunk(&mut self, scope: &RequestScope) -> io::Result<Option<Vec<u8>>> {
        if scope.cancellation().is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "request cancelled",
            ));
        }
        let chunk = self
            .inner
            .next_chunk(scope)
            .inspect_err(|_| scope.cancel_peer_drop())?;
        if let Some(chunk) = &chunk {
            if chunk.is_empty()
                || chunk.len() > self.limits.max_chunk_bytes
                || self.received.saturating_add(chunk.len()) > self.limits.max_request_bytes
            {
                scope.cancel("request body cap exceeded");
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request body cap exceeded",
                ));
            }
            self.received += chunk.len();
        }
        Ok(chunk)
    }
}

struct LimitedResponse<'a> {
    inner: &'a mut dyn ResponseWriter,
    max_chunk: usize,
    trailers: TrailersPolicy,
}
impl ResponseWriter for LimitedResponse<'_> {
    fn write_head(&mut self, head: ResponseHead, scope: &RequestScope) -> io::Result<()> {
        self.inner
            .write_head(head, scope)
            .inspect_err(|_| scope.cancel("response write failure"))
    }
    fn write_chunk(&mut self, chunk: &[u8], scope: &RequestScope) -> io::Result<()> {
        if chunk.is_empty() || chunk.len() > self.max_chunk {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "response chunk outside bounds",
            ));
        }
        self.inner
            .write_chunk(chunk, scope)
            .inspect_err(|_| scope.cancel("response write failure"))
    }
    fn finish(&mut self, trailers: &[Header], scope: &RequestScope) -> io::Result<()> {
        if !trailers.is_empty() && self.trailers == TrailersPolicy::Deny {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "response trailers denied",
            ));
        }
        self.inner
            .finish(trailers, scope)
            .inspect_err(|_| scope.cancel("response write failure"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Body(Vec<Vec<u8>>);
    impl BodyReader for Body {
        fn next_chunk(&mut self, _: &RequestScope) -> io::Result<Option<Vec<u8>>> {
            Ok(if self.0.is_empty() {
                None
            } else {
                Some(self.0.remove(0))
            })
        }
    }
    #[derive(Default)]
    struct Writer {
        chunks: Vec<Vec<u8>>,
        fail_after: usize,
    }
    impl ResponseWriter for Writer {
        fn write_head(&mut self, _: ResponseHead, _: &RequestScope) -> io::Result<()> {
            Ok(())
        }
        fn write_chunk(&mut self, chunk: &[u8], _: &RequestScope) -> io::Result<()> {
            if self.chunks.len() == self.fail_after {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "peer dropped"));
            }
            self.chunks.push(chunk.to_vec());
            Ok(())
        }
        fn finish(&mut self, _: &[Header], _: &RequestScope) -> io::Result<()> {
            Ok(())
        }
    }
    struct Connection {
        head: RequestHead,
        body: Body,
        writer: Writer,
    }
    impl RawConnection for Connection {
        fn parts(&mut self) -> (&RequestHead, &mut dyn BodyReader, &mut dyn ResponseWriter) {
            (&self.head, &mut self.body, &mut self.writer)
        }
    }
    struct Streaming {
        observed: Arc<Mutex<Option<Cancellation>>>,
    }
    impl RawHandler for Streaming {
        fn handle(
            &self,
            _: &RequestHead,
            body: &mut dyn BodyReader,
            out: &mut dyn ResponseWriter,
            scope: &RequestScope,
        ) -> Result<()> {
            *self.observed.lock().unwrap() = Some(scope.cancellation().clone());
            while let Some(chunk) = body
                .next_chunk(scope)
                .map_err(|e| Error::HostError(e.to_string()))?
            {
                out.write_chunk(&chunk, scope)
                    .map_err(|e| Error::HostError(e.to_string()))?;
            }
            Ok(())
        }
    }
    #[test]
    fn streaming_handler_is_backpressured_and_cancelled_on_peer_drop() {
        let observed = Arc::new(Mutex::new(None));
        let server = RawHttpServer::new(
            Streaming {
                observed: Arc::clone(&observed),
            },
            BodyLimits {
                max_request_bytes: 16,
                max_chunk_bytes: 4,
            },
            TrailersPolicy::Deny,
            Duration::from_secs(1),
        )
        .unwrap();
        let mut connection = Connection {
            head: RequestHead {
                method: "POST".into(),
                target: "/mcp".into(),
                headers: vec![("X-A".into(), "1".into()), ("X-A".into(), "2".into())],
                peer: Some("peer".into()),
                local: Some("local".into()),
            },
            body: Body(vec![b"one".to_vec(), b"two".to_vec()]),
            writer: Writer {
                fail_after: 1,
                ..Writer::default()
            },
        };
        assert!(server.serve(&mut connection, &Cancellation::new()).is_err());
        assert_eq!(connection.writer.chunks, vec![b"one".to_vec()]);
        assert!(observed.lock().unwrap().as_ref().unwrap().is_cancelled());
    }
}
