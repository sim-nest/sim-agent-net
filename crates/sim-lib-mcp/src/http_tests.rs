use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use sim_codec_lisp::LispCodecLib;
use sim_codec_mcp::{McpCodecLib, McpEnvelope, McpRequest, McpResponse};
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Cx, DefaultFactory, EagerPolicy, Error, Export,
    Lib, LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, ShapeRef, Stream,
    Symbol, Value, Version, read_eval_capability,
};
use sim_lib_server::{FrameEnvelope, FrameKind, ServerFrame};
use sim_lib_stream_core::StreamPacket;
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpExportFacet, McpHttpAdapter, McpNativeCard, McpProfile, McpSession, mcp_http_capability,
    mcp_tools_call_capability,
};
use sim_kernel::Expr;

#[test]
fn http_initialize_tools_list_and_tools_call_work() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "echo");
    let function = install_tool(&mut cx, symbol.clone(), FixtureBehavior::EchoFirst);
    let capability = CapabilityName::new("fixture.echo.call");
    let session = session_with_tool(symbol, "fixture.echo", capability.clone())
        .with_granted_capability(mcp_http_capability())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(capability);
    let mut adapter = McpHttpAdapter::new(session);

    let initialize = adapter
        .handle_http_envelope(&mut cx, request("init", "initialize", Expr::Nil))
        .unwrap()
        .unwrap();
    assert!(map_field(expect_response(&initialize), "serverInfo").is_some());

    let list = adapter
        .handle_http_envelope(&mut cx, request("list", "tools/list", Expr::Nil))
        .unwrap()
        .unwrap();
    assert!(tools_list_names(expect_response(&list)).contains(&"fixture.echo".to_owned()));

    let call = adapter
        .handle_http_envelope(
            &mut cx,
            request(
                "call",
                "tools/call",
                call_params("fixture.echo", vec![Expr::String("hello".to_owned())]),
            ),
        )
        .unwrap()
        .unwrap();
    assert_eq!(single_text_content(expect_response(&call)), Some("hello"));
    assert_eq!(function.call_count(), 1);
}

#[test]
fn sse_progress_preserves_stream_order() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "streaming");
    let function = install_tool(&mut cx, symbol.clone(), FixtureBehavior::Stream);
    let capability = CapabilityName::new("fixture.streaming.call");
    let session = session_with_tool(symbol, "fixture.streaming", capability.clone())
        .with_granted_capability(mcp_http_capability())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(capability);
    let mut adapter = McpHttpAdapter::new(session);

    let replies = adapter
        .handle_sse_envelope(
            &mut cx,
            request(
                "stream",
                "tools/call",
                call_params_with_token("fixture.streaming", Expr::String("tok".to_owned())),
            ),
        )
        .unwrap();

    assert_eq!(replies.len(), 3);
    assert_eq!(
        progress_payload(&replies[0]),
        Some(&Expr::String("first".to_owned()))
    );
    assert_eq!(
        progress_payload(&replies[1]),
        Some(&Expr::String("second".to_owned()))
    );
    assert!(matches!(replies[2], McpEnvelope::Response(_)));
    assert_eq!(function.call_count(), 1);
    assert_eq!(function.next_count(), 2);
}

#[test]
fn websocket_frames_round_trip_through_server_codec_and_router() {
    let mut cx = cx();
    let session =
        McpSession::new("ws", McpProfile::all()).with_granted_capability(mcp_http_capability());
    let mut adapter = McpHttpAdapter::new(session);
    let frame = adapter
        .frame_from_envelope(
            &mut cx,
            &request("ping", "ping", Expr::Nil),
            Default::default(),
        )
        .unwrap();
    let bytes = McpHttpAdapter::encode_frame(&frame).unwrap();
    let frame = McpHttpAdapter::decode_frame(&bytes).unwrap();

    let replies = adapter.handle_websocket_frames(&mut cx, [frame]).unwrap();

    assert_eq!(replies.len(), 1);
    let reply = adapter.envelope_from_frame(&mut cx, &replies[0]).unwrap();
    assert!(matches!(reply, McpEnvelope::Response(McpResponse { .. })));
}

#[test]
fn http_frame_payload_rejects_lisp_read_eval() {
    let mut cx = cx_with_lisp();
    let session = McpSession::new("http-read-eval", McpProfile::all())
        .with_granted_capability(mcp_http_capability());
    let mut adapter = McpHttpAdapter::new(session);
    let frame = ServerFrame::new(
        Symbol::qualified("codec", "lisp"),
        FrameKind::Request,
        FrameEnvelope::default(),
        b"#. 1".to_vec(),
    );

    let err = adapter.handle_http_frame(&mut cx, frame).unwrap_err();

    assert_read_eval_denied(err);
}

#[test]
fn network_gate_is_enforced() {
    let mut cx = cx();
    let mut adapter = McpHttpAdapter::new(McpSession::new("denied", McpProfile::all()));

    let err = adapter
        .handle_http_envelope(&mut cx, request("ping", "ping", Expr::Nil))
        .unwrap_err();

    match err {
        Error::CapabilityDenied { capability } if capability == mcp_http_capability() => {}
        other => panic!("expected mcp.http capability denial, got {other:?}"),
    }
}

fn install_tool(cx: &mut Cx, symbol: Symbol, behavior: FixtureBehavior) -> Arc<FixtureFunction> {
    let function = Arc::new(FixtureFunction::new(behavior));
    cx.load_lib(&FixtureLib {
        id: Symbol::qualified("mcp-http-test", symbol.to_string()),
        symbol,
        function: function.clone(),
    })
    .unwrap();
    function
}

fn session_with_tool(symbol: Symbol, name: &str, capability: CapabilityName) -> McpSession {
    McpSession::new("mcp-http", McpProfile::all()).with_native_cards(vec![
        McpNativeCard::new(symbol, "Fixture MCP HTTP native tool")
            .with_shapes(any_shape("tool-args"), any_shape("tool-result"))
            .with_capability(capability)
            .exported(McpExportFacet::tool().with_name(name.to_owned())),
    ])
}

#[derive(Clone, Copy)]
enum FixtureBehavior {
    EchoFirst,
    Stream,
}

struct FixtureFunction {
    calls: Arc<AtomicUsize>,
    nexts: Arc<AtomicUsize>,
    behavior: FixtureBehavior,
}

impl FixtureFunction {
    fn new(behavior: FixtureBehavior) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            nexts: Arc::new(AtomicUsize::new(0)),
            behavior,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn next_count(&self) -> usize {
        self.nexts.load(Ordering::SeqCst)
    }
}

impl Object for FixtureFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-http-test-function>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for FixtureFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for FixtureFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behavior {
            FixtureBehavior::EchoFirst => args
                .values()
                .first()
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| cx.factory().nil()),
            FixtureBehavior::Stream => cx.factory().opaque(Arc::new(FixtureStream::new(
                vec![
                    StreamPacket::data(
                        Symbol::qualified("stream/data", "fixture"),
                        Expr::String("first".to_owned()),
                    )
                    .to_expr(),
                    StreamPacket::data(
                        Symbol::qualified("stream/data", "fixture"),
                        Expr::String("second".to_owned()),
                    )
                    .to_expr(),
                ],
                self.nexts.clone(),
            ))),
        }
    }
}

struct FixtureStream {
    items: Mutex<VecDeque<Expr>>,
    nexts: Arc<AtomicUsize>,
}

impl FixtureStream {
    fn new(items: Vec<Expr>, nexts: Arc<AtomicUsize>) -> Self {
        Self {
            items: Mutex::new(items.into()),
            nexts,
        }
    }
}

impl Object for FixtureStream {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-http-test-stream>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for FixtureStream {
    fn as_stream(&self) -> Option<&dyn Stream> {
        Some(self)
    }

    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Map(vec![(
            Expr::String("kind".to_owned()),
            Expr::String("fixture-stream".to_owned()),
        )]))
    }
}

impl Stream for FixtureStream {
    fn next(&self, cx: &mut Cx) -> Result<Option<Value>> {
        let item = self
            .items
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture MCP HTTP stream"))?
            .pop_front();
        match item {
            Some(expr) => {
                self.nexts.fetch_add(1, Ordering::SeqCst);
                Ok(Some(cx.factory().expr(expr)?))
            }
            None => Ok(None),
        }
    }
}

struct FixtureLib {
    id: Symbol,
    symbol: Symbol,
    function: Arc<FixtureFunction>,
}

impl Lib for FixtureLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: self.id.clone(),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: self.symbol.clone(),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            self.symbol.clone(),
            cx.factory().opaque(self.function.clone())?,
        )?;
        Ok(())
    }
}

fn request(id: &str, method: &str, params: Expr) -> McpEnvelope {
    McpEnvelope::Request(McpRequest {
        id: Expr::String(id.to_owned()),
        method: method.to_owned(),
        params,
    })
}

fn call_params(name: &str, arguments: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        field("name", Expr::String(name.to_owned())),
        field("arguments", Expr::List(arguments)),
    ])
}

fn call_params_with_token(name: &str, token: Expr) -> Expr {
    Expr::Map(vec![
        field("name", Expr::String(name.to_owned())),
        field("arguments", Expr::List(Vec::new())),
        field("_meta", Expr::Map(vec![field("progressToken", token)])),
    ])
}

fn expect_response(envelope: &McpEnvelope) -> &Expr {
    let McpEnvelope::Response(McpResponse { result, .. }) = envelope else {
        panic!("expected MCP response");
    };
    result
}

fn tools_list_names(result: &Expr) -> Vec<String> {
    match map_field(result, "tools") {
        Some(Expr::List(tools)) => tools
            .iter()
            .filter_map(|tool| map_field(tool, "name"))
            .filter_map(|name| match name {
                Expr::String(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn single_text_content(result: &Expr) -> Option<&str> {
    let content = match map_field(result, "content") {
        Some(Expr::List(items)) if items.len() == 1 => items.first()?,
        _ => return None,
    };
    match map_field(content, "text") {
        Some(Expr::String(text)) => Some(text.as_str()),
        _ => None,
    }
}

fn progress_payload(envelope: &McpEnvelope) -> Option<&Expr> {
    let McpEnvelope::Notification(notification) = envelope else {
        return None;
    };
    let data = map_field(&notification.params, "data")?;
    map_field(data, "payload")
}

use sim_value::access::field_any as map_field;

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-http-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

fn cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let codec = McpCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&codec).unwrap();
    cx
}

fn cx_with_lisp() -> Cx {
    let mut cx = cx();
    let codec = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&codec).unwrap();
    cx
}

fn assert_read_eval_denied(error: Error) {
    assert!(
        matches!(error, Error::CapabilityDenied { ref capability } if capability == &read_eval_capability()),
        "expected read-eval denial, got {error:?}",
    );
}

use sim_value::build::entry as field;
