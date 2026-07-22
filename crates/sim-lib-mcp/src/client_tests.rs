use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use sim_codec_mcp::{McpEnvelope, McpRequest, McpResponse};
use sim_kernel::{
    Args, Callable, Cx, Error, Expr, Object, ObjectCompat, Result, ShapeRef, Symbol, Value,
};
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpClient, McpClientAllowPolicy, McpClientCassettePeer, McpClientPeer, McpExportFacet,
    McpNativeCard, McpProfile, McpRouter, McpSession, mcp_client_capability,
    mcp_prompts_get_capability, mcp_resources_read_capability, mcp_tools_call_capability,
};

#[test]
fn inbound_client_imports_only_explicitly_allowed_descriptors_as_skill_cards() {
    let mut cx = cx();
    cx.grant(mcp_client_capability());
    let peer = Arc::new(RecordingPeer::new(foreign_router()));
    let client = McpClient::new("bridge", peer);

    let cards = client
        .import_allowed(
            &mut cx,
            &McpClientAllowPolicy::new()
                .allow_tool("foreign.echo")
                .allow_resource("sim://browse/foreign/readme")
                .allow_prompt("foreign.prompt"),
        )
        .unwrap();

    let mut ids = cards
        .iter()
        .map(|card| card.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![
            "bridge.prompt.foreign.prompt",
            "bridge.resource.sim-browse.foreign.readme",
            "bridge.tool.foreign.echo",
        ]
    );
    assert_eq!(
        tool_names(&mcp_response_result(request(
            &mut cx,
            McpRouter::new(session_with_card_capabilities(&cards)),
            "tools/list",
            Expr::Nil,
        ))),
        vec!["bridge.tool.foreign.echo"]
    );
    assert_eq!(
        resource_uris(&mcp_response_result(request(
            &mut cx,
            McpRouter::new(session_with_card_capabilities(&cards)),
            "resources/list",
            Expr::Nil,
        ))),
        vec!["skill://bridge.resource.sim-browse.foreign.readme"]
    );
    assert_eq!(
        prompt_names(&mcp_response_result(request(
            &mut cx,
            McpRouter::new(session_with_card_capabilities(&cards)),
            "prompts/list",
            Expr::Nil,
        ))),
        vec!["bridge.prompt.foreign.prompt"]
    );
}

fn session_with_card_capabilities(cards: &[sim_lib_skill::SkillCard]) -> McpSession {
    cards.iter().fold(
        McpSession::new("local", McpProfile::all()),
        |mut session, card| {
            for capability in &card.capabilities {
                session = session.with_granted_capability(capability.clone());
            }
            session
        },
    )
}

#[test]
fn inbound_client_blocks_descriptors_without_explicit_allow_policy() {
    let mut cx = cx();
    cx.grant(mcp_client_capability());
    let client = McpClient::new("bridge", Arc::new(RecordingPeer::new(foreign_router())));

    let cards = client
        .import_allowed(&mut cx, &McpClientAllowPolicy::new())
        .unwrap();

    assert!(cards.is_empty());
    let registry = sim_lib_skill::skill_registry(&mut cx).unwrap();
    assert!(registry.cards().unwrap().is_empty());
}

#[test]
fn transitive_re_served_foreign_tool_uses_one_router_path() {
    let mut cx = cx();
    cx.grant(mcp_client_capability());
    cx.grant(sim_lib_skill::skill_call_capability());
    let counter = install_echo_counter(&mut cx, Symbol::qualified("foreign", "echo"));
    let peer = Arc::new(RecordingPeer::new(foreign_router()));
    let client = McpClient::new("bridge", peer.clone());
    let cards = client
        .import_allowed(
            &mut cx,
            &McpClientAllowPolicy::new().allow_tool("foreign.echo"),
        )
        .unwrap();
    let imported_capability = sim_lib_skill::skill_specific_call_capability(&cards[0].id);
    cx.grant(imported_capability.clone());
    let session = McpSession::new("local", McpProfile::all())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(imported_capability);

    let result = mcp_response_result(request(
        &mut cx,
        McpRouter::new(session),
        "tools/call",
        Expr::Map(vec![
            field("name", Expr::String("bridge.tool.foreign.echo".to_owned())),
            field(
                "arguments",
                Expr::List(vec![Expr::String("through-router".to_owned())]),
            ),
        ]),
    ));

    assert_eq!(counter.call_count(), 1);
    assert_eq!(
        single_text_content(&result),
        Some("through-router".to_owned())
    );
    assert_eq!(
        peer.methods().unwrap(),
        vec![
            "initialize",
            "tools/list",
            "resources/list",
            "prompts/list",
            "tools/call"
        ]
    );
}

#[test]
fn inbound_client_cassette_replays_foreign_discovery() {
    let mut cx = cx();
    cx.grant(mcp_client_capability());
    let peer = Arc::new(McpClientCassettePeer::new(vec![
        ("initialize".to_owned(), initialize_result()),
        ("tools/list".to_owned(), tools_list_result()),
        (
            "resources/list".to_owned(),
            Expr::Map(vec![field("resources", Expr::List(Vec::new()))]),
        ),
        (
            "prompts/list".to_owned(),
            Expr::Map(vec![field("prompts", Expr::List(Vec::new()))]),
        ),
    ]));
    let client = McpClient::new("cassette", peer.clone());

    let cards = client
        .import_allowed(
            &mut cx,
            &McpClientAllowPolicy::new().allow_tool("foreign.echo"),
        )
        .unwrap();

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].id, "cassette.tool.foreign.echo");
    assert_eq!(peer.remaining().unwrap(), 0);
}

struct RecordingPeer {
    router: Mutex<McpRouter>,
    methods: Mutex<Vec<String>>,
}

impl RecordingPeer {
    fn new(router: McpRouter) -> Self {
        Self {
            router: Mutex::new(router),
            methods: Mutex::new(Vec::new()),
        }
    }

    fn methods(&self) -> Result<Vec<&'static str>> {
        let methods = self
            .methods
            .lock()
            .map_err(|_| Error::PoisonedLock("recording peer methods"))?
            .iter()
            .map(|method| match method.as_str() {
                "initialize" => "initialize",
                "tools/list" => "tools/list",
                "resources/list" => "resources/list",
                "prompts/list" => "prompts/list",
                "tools/call" => "tools/call",
                _ => "other",
            })
            .collect();
        Ok(methods)
    }
}

impl McpClientPeer for RecordingPeer {
    fn exchange(&self, cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope> {
        if let McpEnvelope::Request(request) = &envelope {
            self.methods
                .lock()
                .map_err(|_| Error::PoisonedLock("recording peer methods"))?
                .push(request.method.clone());
        }
        self.router
            .lock()
            .map_err(|_| Error::PoisonedLock("recording peer router"))?
            .handle(cx, envelope)?
            .ok_or_else(|| Error::Eval("foreign router returned no response".to_owned()))
    }
}

fn foreign_router() -> McpRouter {
    let session = McpSession::new("foreign", McpProfile::all())
        .with_native_cards(vec![
            native_tool(),
            native_resource(),
            native_prompt(),
            blocked_tool(),
        ])
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(mcp_resources_read_capability())
        .with_granted_capability(mcp_prompts_get_capability());
    McpRouter::new(session)
}

fn native_tool() -> McpNativeCard {
    McpNativeCard::new(Symbol::qualified("foreign", "echo"), "Foreign echo")
        .with_shapes(any_shape("args"), any_shape("result"))
        .exported(McpExportFacet::tool().with_name("foreign.echo"))
}

fn blocked_tool() -> McpNativeCard {
    McpNativeCard::new(Symbol::qualified("foreign", "blocked"), "Blocked")
        .with_shapes(any_shape("args"), any_shape("result"))
        .exported(McpExportFacet::tool().with_name("foreign.blocked"))
}

fn native_resource() -> McpNativeCard {
    McpNativeCard::new(Symbol::qualified("foreign", "readme"), "Foreign readme")
        .with_shapes(any_shape("args"), any_shape("result"))
        .exported(
            McpExportFacet::resource()
                .with_name("foreign.readme")
                .with_uri("sim://browse/foreign/readme"),
        )
}

fn native_prompt() -> McpNativeCard {
    McpNativeCard::new(Symbol::qualified("foreign", "prompt"), "Foreign prompt")
        .with_shapes(any_shape("args"), any_shape("result"))
        .exported(McpExportFacet::prompt().with_name("foreign.prompt"))
}

fn request(cx: &mut Cx, mut router: McpRouter, method: &str, params: Expr) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(method.to_owned()),
                method: method.to_owned(),
                params,
            }),
        )
        .unwrap()
        .unwrap()
}

fn mcp_response_result(envelope: McpEnvelope) -> Expr {
    let McpEnvelope::Response(McpResponse { result, .. }) = envelope else {
        panic!("expected response");
    };
    result
}

fn initialize_result() -> Expr {
    Expr::Map(vec![
        field("protocolVersion", Expr::String("2025-03-26".to_owned())),
        field("capabilities", Expr::Map(Vec::new())),
    ])
}

fn tools_list_result() -> Expr {
    Expr::Map(vec![field(
        "tools",
        Expr::List(vec![Expr::Map(vec![
            field("name", Expr::String("foreign.echo".to_owned())),
            field("description", Expr::String("Foreign echo".to_owned())),
            field("inputSchema", Expr::Map(Vec::new())),
        ])]),
    )])
}

fn install_echo_counter(cx: &mut Cx, symbol: Symbol) -> Arc<EchoCounter> {
    let counter = Arc::new(EchoCounter::default());
    let callable = cx.factory().opaque(counter.clone()).unwrap();
    cx.registry_mut()
        .register_function_value(symbol, callable)
        .unwrap();
    counter
}

#[derive(Default)]
struct EchoCounter {
    calls: AtomicUsize,
}

impl EchoCounter {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Object for EchoCounter {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-client-test-echo>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for EchoCounter {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for EchoCounter {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        args.values()
            .first()
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| cx.factory().nil())
    }
}

fn tool_names(result: &Expr) -> Vec<String> {
    string_fields_in_list(result, "tools", "name")
}

fn resource_uris(result: &Expr) -> Vec<String> {
    string_fields_in_list(result, "resources", "uri")
}

fn prompt_names(result: &Expr) -> Vec<String> {
    string_fields_in_list(result, "prompts", "name")
}

fn string_fields_in_list(result: &Expr, list: &str, field_name: &str) -> Vec<String> {
    let Some(Expr::List(items)) = field_value(result, list) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| field_value(item, field_name))
        .filter_map(|value| match value {
            Expr::String(value) => Some(value.clone()),
            _ => None,
        })
        .collect()
}

fn single_text_content(result: &Expr) -> Option<String> {
    let Expr::List(items) = field_value(result, "content")? else {
        return None;
    };
    let content = items.first()?;
    match field_value(content, "text")? {
        Expr::String(text) => Some(text.clone()),
        _ => None,
    }
}

use sim_value::access::field_any as field_value;

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-client-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_kernel::testing::eager_cx as cx;

use sim_value::build::entry as field;
