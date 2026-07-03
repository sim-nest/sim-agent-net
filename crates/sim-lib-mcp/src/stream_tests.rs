use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use sim_codec_mcp::{McpEnvelope, McpNotification, McpRequest, McpResponse};
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Cx, Error, Export, Lib, LibManifest, LibTarget,
    Linker, LoadCx, Object, ObjectCompat, Result, ShapeRef, Stream, Symbol, Value, Version,
};
use sim_lib_stream_core::StreamPacket;
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession, mcp_cancelled_data_kind,
    mcp_tools_call_capability,
};
use sim_kernel::Expr;

#[test]
fn tools_call_with_progress_token_emits_ordered_notifications_and_final_response() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "streaming");
    let function = install_streaming_tool(&mut cx, symbol.clone());
    let capability = CapabilityName::new("fixture.streaming.call");
    let session = session_with_streaming_tool(symbol, "fixture.streaming", capability.clone())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(capability);
    let mut router = McpRouter::new(session);

    let replies = router
        .handle_many(
            &mut cx,
            request(
                Expr::String("stream-1".to_owned()),
                "tools/call",
                call_params_with_token("fixture.streaming", Some(Expr::String("tok-1".to_owned()))),
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
    assert_eq!(
        progress_token(&replies[0]),
        Some(&Expr::String("tok-1".to_owned()))
    );
    assert_final_success(&replies[2]);
    assert_eq!(function.call_count(), 1);
    assert_eq!(function.next_count(), 2);
}

#[test]
fn tools_call_without_progress_token_records_stream_but_emits_only_final_response() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "quiet-streaming");
    let function = install_streaming_tool(&mut cx, symbol.clone());
    let capability = CapabilityName::new("fixture.quiet-streaming.call");
    let session = session_with_streaming_tool(symbol, "fixture.quiet", capability.clone())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(capability);
    let mut router = McpRouter::new(session);

    let replies = router
        .handle_many(
            &mut cx,
            request(
                Expr::String("stream-2".to_owned()),
                "tools/call",
                call_params_with_token("fixture.quiet", None),
            ),
        )
        .unwrap();

    assert_eq!(replies.len(), 1);
    assert_final_success(&replies[0]);
    assert_eq!(function.call_count(), 1);
    assert_eq!(function.next_count(), 2);
    assert_eq!(router.session().stream_packets().len(), 2);
}

#[test]
fn cancellation_notification_marks_active_request_and_records_data_packet() {
    let mut cx = cx();
    let mut router = McpRouter::fixture();
    let request_id = Expr::String("cancel-1".to_owned());
    router.session_mut().begin_request(&request_id);

    let replies = router
        .handle_many(
            &mut cx,
            McpEnvelope::Notification(McpNotification {
                method: "notifications/cancelled".to_owned(),
                params: Expr::Map(vec![
                    field("requestId", request_id.clone()),
                    field("reason", Expr::String("client stopped".to_owned())),
                ]),
            }),
        )
        .unwrap();

    assert!(replies.is_empty());
    assert!(router.session().request_cancelled(&request_id));
    let [StreamPacket::Data(packet)] = router.session().stream_packets() else {
        panic!("expected one cancellation data packet");
    };
    assert_eq!(packet.kind, mcp_cancelled_data_kind());
    assert_eq!(map_field(&packet.payload, "requestId"), Some(&request_id));
    assert_eq!(
        map_field(&packet.payload, "reason"),
        Some(&Expr::String("client stopped".to_owned()))
    );
}

fn install_streaming_tool(cx: &mut Cx, symbol: Symbol) -> Arc<StreamingFunction> {
    let function = Arc::new(StreamingFunction::new());
    cx.load_lib(&StreamingLib {
        id: Symbol::qualified("mcp-stream-test", symbol.to_string()),
        symbol,
        function: function.clone(),
    })
    .unwrap();
    function
}

fn session_with_streaming_tool(
    symbol: Symbol,
    name: &str,
    capability: CapabilityName,
) -> McpSession {
    McpSession::new("mcp-stream", McpProfile::all()).with_native_cards(vec![
        McpNativeCard::new(symbol, "Fixture streaming MCP native tool")
            .with_shapes(any_shape("stream-args"), any_shape("stream-result"))
            .with_capability(capability)
            .exported(McpExportFacet::tool().with_name(name.to_owned())),
    ])
}

struct StreamingFunction {
    calls: Arc<AtomicUsize>,
    nexts: Arc<AtomicUsize>,
}

impl StreamingFunction {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            nexts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn next_count(&self) -> usize {
        self.nexts.load(Ordering::SeqCst)
    }
}

impl Object for StreamingFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-stream-test-function>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for StreamingFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for StreamingFunction {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        cx.factory().opaque(Arc::new(FixtureStream::new(
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
        )))
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
        Ok("#<mcp-stream-test-stream>".to_owned())
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
        Ok(Expr::Map(vec![
            (
                Expr::String("kind".to_owned()),
                Expr::String("fixture-stream".to_owned()),
            ),
            (
                Expr::String("done".to_owned()),
                Expr::Bool(
                    self.items
                        .lock()
                        .map(|items| items.is_empty())
                        .unwrap_or(true),
                ),
            ),
        ]))
    }
}

impl Stream for FixtureStream {
    fn next(&self, cx: &mut Cx) -> Result<Option<Value>> {
        let item = self
            .items
            .lock()
            .map_err(|_| Error::PoisonedLock("fixture MCP stream"))?
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

struct StreamingLib {
    id: Symbol,
    symbol: Symbol,
    function: Arc<StreamingFunction>,
}

impl Lib for StreamingLib {
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

fn request(id: Expr, method: &str, params: Expr) -> McpEnvelope {
    McpEnvelope::Request(McpRequest {
        id,
        method: method.to_owned(),
        params,
    })
}

fn call_params_with_token(name: &str, token: Option<Expr>) -> Expr {
    let mut fields = vec![
        field("name", Expr::String(name.to_owned())),
        field("arguments", Expr::List(Vec::new())),
    ];
    if let Some(token) = token {
        fields.push(field(
            "_meta",
            Expr::Map(vec![field("progressToken", token)]),
        ));
    }
    Expr::Map(fields)
}

fn assert_final_success(envelope: &McpEnvelope) {
    let McpEnvelope::Response(McpResponse { result, .. }) = envelope else {
        panic!("expected final MCP response");
    };
    assert_eq!(map_field(result, "isError"), Some(&Expr::Bool(false)));
}

fn progress_payload(envelope: &McpEnvelope) -> Option<&Expr> {
    let params = progress_params(envelope)?;
    let data = map_field(params, "data")?;
    map_field(data, "payload")
}

fn progress_token(envelope: &McpEnvelope) -> Option<&Expr> {
    progress_params(envelope).and_then(|params| map_field(params, "progressToken"))
}

fn progress_params(envelope: &McpEnvelope) -> Option<&Expr> {
    let McpEnvelope::Notification(notification) = envelope else {
        return None;
    };
    (notification.method == "notifications/progress").then_some(&notification.params)
}

use sim_value::access::field_any as map_field;

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-stream-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_kernel::testing::eager_cx as cx;

use sim_value::build::entry as field;
