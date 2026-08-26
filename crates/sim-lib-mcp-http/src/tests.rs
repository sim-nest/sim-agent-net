use super::*;
use sim_codec_mcp::{McpCodecLib, McpRequest, McpResponse};
use sim_kernel::{DefaultFactory, EagerPolicy};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Identity;
impl IdentityProvider for Identity {
    fn identify(&self, _head: &RequestHead) -> std::result::Result<RequestIdentity, AuthRejection> {
        Ok(RequestIdentity {
            principal: Principal::new("test-principal"),
            grants: CapabilitySet::new(),
        })
    }
}
struct Clock;
impl HttpClock for Clock {
    fn http_date(&self) -> String {
        "Sun, 23 Aug 2026 12:00:00 GMT".into()
    }
    fn keepalive(&self) -> Option<u64> {
        Some(7)
    }
}
struct Dispatch {
    calls: Arc<AtomicUsize>,
}
impl McpDispatch for Dispatch {
    fn dispatch(
        &self,
        context: &RequestContext,
        cancellation: &Cancellation,
        envelope: McpEnvelope,
    ) -> Result<Vec<McpEnvelope>> {
        assert_eq!(context.principal().subject(), "test-principal");
        assert!(!cancellation.is_cancelled());
        self.calls.fetch_add(1, Ordering::SeqCst);
        match envelope {
            McpEnvelope::Request(request) => Ok(vec![McpEnvelope::Response(McpResponse {
                id: request.id,
                result: Expr::String("typed".into()),
            })]),
            McpEnvelope::Notification(_) => Ok(Vec::new()),
            _ => unreachable!(),
        }
    }
}
struct Body {
    chunks: Vec<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}
impl BodyReader for Body {
    fn next_chunk(&mut self, _scope: &RequestScope) -> io::Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(if self.chunks.is_empty() {
            None
        } else {
            Some(self.chunks.remove(0))
        })
    }
}
#[derive(Default)]
struct Writer {
    head: Option<ResponseHead>,
    chunks: Vec<Vec<u8>>,
    fail_after_head: bool,
}
impl ResponseWriter for Writer {
    fn write_head(&mut self, head: ResponseHead, _scope: &RequestScope) -> io::Result<()> {
        self.head = Some(head);
        Ok(())
    }
    fn write_chunk(&mut self, chunk: &[u8], _scope: &RequestScope) -> io::Result<()> {
        if self.fail_after_head {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "peer drop"));
        }
        self.chunks.push(chunk.to_vec());
        Ok(())
    }
    fn finish(&mut self, _trailers: &[(String, String)], _scope: &RequestScope) -> io::Result<()> {
        Ok(())
    }
}
fn codec_cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(0x4d43_5048),
    );
    let codec = McpCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&codec).unwrap();
    cx
}
fn request(method: &str) -> McpEnvelope {
    McpEnvelope::Request(McpRequest {
        id: Expr::String("1".into()),
        method: method.into(),
        params: Expr::Nil,
    })
}
fn bytes(envelope: &McpEnvelope) -> Vec<u8> {
    encode(&Mutex::new(codec_cx()), envelope).unwrap()
}
fn head(method: &str, accept: &str) -> RequestHead {
    RequestHead {
        method: method.into(),
        target: "/mcp".into(),
        headers: vec![
            ("Content-Type".into(), JSON.into()),
            ("Accept".into(), accept.into()),
            ("MCP-Protocol-Version".into(), PROTOCOL.into()),
            ("MCP-Method".into(), "ping".into()),
        ],
        peer: Some("127.0.0.1:1".into()),
        local: Some("127.0.0.1:2".into()),
    }
}
fn handler(calls: Arc<AtomicUsize>) -> McpHttpHandler<Dispatch, Identity, Clock> {
    McpHttpHandler::new(
        ServerPolicy::new("/mcp", OriginPolicy::LoopbackOnly, 4096).unwrap(),
        Dispatch { calls },
        Identity,
        Clock,
        codec_cx(),
    )
}
fn scope() -> RequestScope {
    RequestScope::child(&Cancellation::new(), std::time::Duration::from_secs(1))
}

#[test]
fn method_and_origin_reject_before_body_read() {
    for (mut request, status) in [
        (head("GET", JSON), 405),
        (
            RequestHead {
                headers: vec![("Origin".into(), "https://evil.example".into())],
                ..head("POST", JSON)
            },
            403,
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let reads = Arc::new(AtomicUsize::new(0));
        let mut body = Body {
            chunks: vec![vec![1]],
            reads: reads.clone(),
        };
        let mut writer = Writer::default();
        handler(calls.clone())
            .handle(&request, &mut body, &mut writer, &scope())
            .unwrap();
        assert_eq!(writer.head.unwrap().status, status);
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        request.target.clear();
    }
}
#[test]
fn json_and_sse_have_same_typed_result_and_no_session_headers() {
    for accept in [JSON, SSE] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut body = Body {
            chunks: vec![bytes(&request("ping"))],
            reads: Arc::new(AtomicUsize::new(0)),
        };
        let mut writer = Writer::default();
        handler(calls.clone())
            .handle(&head("POST", accept), &mut body, &mut writer, &scope())
            .unwrap();
        assert_eq!(writer.head.as_ref().unwrap().status, 200);
        assert!(
            !writer
                .head
                .as_ref()
                .unwrap()
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("mcp-session-id"))
        );
        let wire = writer.chunks.concat();
        let decoded = if accept == JSON {
            decode(&Mutex::new(codec_cx()), wire).unwrap()
        } else {
            parse_sse(&wire, &Mutex::new(codec_cx()))
                .unwrap()
                .pop()
                .unwrap()
        };
        let McpEnvelope::Response(response) = decoded else {
            panic!()
        };
        assert_eq!(response.result, Expr::String("typed".into()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn notification_is_202_and_projection_conflict_never_dispatches() {
    let notification = McpEnvelope::Notification(sim_codec_mcp::McpNotification {
        method: "changed".into(),
        params: Expr::Nil,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = head("POST", JSON);
    h.headers
        .iter_mut()
        .find(|(n, _)| n == "MCP-Method")
        .unwrap()
        .1 = "changed".into();
    let mut body = Body {
        chunks: vec![bytes(&notification)],
        reads: Arc::new(AtomicUsize::new(0)),
    };
    let mut writer = Writer::default();
    handler(calls.clone())
        .handle(&h, &mut body, &mut writer, &scope())
        .unwrap();
    assert_eq!(writer.head.unwrap().status, 202);
    assert!(writer.chunks.is_empty());
    let mut conflict = head("POST", JSON);
    conflict.headers.push(("mcp-method".into(), "other".into()));
    let mut body = Body {
        chunks: vec![bytes(&request("ping"))],
        reads: Arc::new(AtomicUsize::new(0)),
    };
    let mut writer = Writer::default();
    handler(calls.clone())
        .handle(&conflict, &mut body, &mut writer, &scope())
        .unwrap();
    assert_eq!(writer.head.unwrap().status, 400);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[test]
fn response_drop_cancels_only_its_scope() {
    let calls = Arc::new(AtomicUsize::new(0));
    let parent = Cancellation::new();
    let first = RequestScope::child(&parent, std::time::Duration::from_secs(1));
    let second = RequestScope::child(&parent, std::time::Duration::from_secs(1));
    let mut body = Body {
        chunks: vec![bytes(&request("ping"))],
        reads: Arc::new(AtomicUsize::new(0)),
    };
    let mut writer = Writer {
        fail_after_head: true,
        ..Default::default()
    };
    assert!(
        handler(calls)
            .handle(&head("POST", JSON), &mut body, &mut writer, &first)
            .is_err()
    );
    assert!(first.cancellation().is_cancelled());
    assert!(!second.cancellation().is_cancelled());
}
