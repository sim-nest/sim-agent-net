use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

// conformance: Streamable HTTP preserves stateless MCP request rejection and endpoint separation.

use sim_cancel::Cancellation;
use sim_codec_mcp::McpEnvelope;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, HandleSeed, Result};
use sim_lib_mcp::{Principal, RequestContext};
use sim_lib_mcp_http::{
    HttpClock, IdentityProvider, McpDispatch, McpHttpHandler, OriginPolicy, RequestIdentity,
    ServerPolicy,
};
use sim_lib_server::{
    BodyReader, RawHandler, RequestHead, RequestScope, ResponseHead, ResponseWriter,
};

struct Identity;
impl IdentityProvider for Identity {
    fn identify(&self, _head: &RequestHead) -> Result<RequestIdentity> {
        Ok(RequestIdentity {
            principal: Principal::new("integration-principal"),
        })
    }
}

struct Clock;
impl HttpClock for Clock {
    fn http_date(&self) -> String {
        "Sun, 23 Aug 2026 12:00:00 GMT".into()
    }

    fn keepalive(&self) -> Option<u64> {
        None
    }
}

struct Dispatch(Arc<AtomicUsize>);
impl McpDispatch for Dispatch {
    fn dispatch(
        &self,
        _context: &RequestContext,
        _cancellation: &Cancellation,
        _envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

struct Body {
    reads: Arc<AtomicUsize>,
}
impl BodyReader for Body {
    fn next_chunk(&mut self, _scope: &RequestScope) -> io::Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Vec::from(b"{}")))
    }
}

#[derive(Default)]
struct Writer {
    head: Option<ResponseHead>,
}
impl ResponseWriter for Writer {
    fn write_head(&mut self, head: ResponseHead, _scope: &RequestScope) -> io::Result<()> {
        self.head = Some(head);
        Ok(())
    }

    fn write_chunk(&mut self, _chunk: &[u8], _scope: &RequestScope) -> io::Result<()> {
        Ok(())
    }

    fn finish(&mut self, _trailers: &[(String, String)], _scope: &RequestScope) -> io::Result<()> {
        Ok(())
    }
}

fn host_cx() -> Cx {
    Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x4854_5450),
    )
}

fn request(method: &str, headers: Vec<(String, String)>) -> RequestHead {
    RequestHead {
        method: method.into(),
        target: "/mcp".into(),
        headers,
        peer: Some("127.0.0.1:1".into()),
        local: Some("127.0.0.1:2".into()),
    }
}

fn scope() -> RequestScope {
    RequestScope::child(&Cancellation::new(), std::time::Duration::from_secs(1))
}

#[test]
fn method_and_origin_policy_reject_before_body_or_dispatch() {
    let dispatches = Arc::new(AtomicUsize::new(0));
    let handler = McpHttpHandler::new(
        ServerPolicy::new(
            "/mcp",
            OriginPolicy::Exact(vec!["https://allowed.example".into()]),
            4096,
        )
        .unwrap(),
        Dispatch(dispatches.clone()),
        Identity,
        Clock,
        host_cx(),
    );

    for (head, status) in [
        (
            request(
                "GET",
                vec![("Origin".into(), "https://allowed.example".into())],
            ),
            405,
        ),
        (
            request(
                "POST",
                vec![("Origin".into(), "https://other.example".into())],
            ),
            403,
        ),
    ] {
        let reads = Arc::new(AtomicUsize::new(0));
        let mut body = Body {
            reads: reads.clone(),
        };
        let mut writer = Writer::default();

        handler
            .handle(&head, &mut body, &mut writer, &scope())
            .unwrap();

        assert_eq!(writer.head.unwrap().status, status);
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn legacy_endpoint_is_explicit_and_distinct() {
    let policy = ServerPolicy::new("/mcp", OriginPolicy::LoopbackOnly, 4096)
        .unwrap()
        .with_legacy_endpoint("/legacy-mcp")
        .unwrap();
    assert_eq!(policy.endpoint, "/mcp");
    assert_eq!(policy.legacy_endpoint.as_deref(), Some("/legacy-mcp"));

    let err = ServerPolicy::new("/mcp", OriginPolicy::LoopbackOnly, 4096)
        .unwrap()
        .with_legacy_endpoint("/mcp")
        .unwrap_err();
    assert!(err.to_string().contains("legacy endpoint"));

    let err = ServerPolicy::new("relative", OriginPolicy::LoopbackOnly, 4096).unwrap_err();
    assert!(matches!(err, sim_kernel::Error::Eval(_)));
}
