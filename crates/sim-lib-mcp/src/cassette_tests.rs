#[cfg(feature = "sampling")]
use std::sync::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use sim_codec_mcp::{
    EXECUTION_ERROR, McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse, RATE_LIMITED,
};
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Cx, DefaultFactory, EagerPolicy, Export, Expr, Lib,
    LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, ShapeRef, Symbol, Value,
    Version,
};
use sim_shape::{AnyShape, shape_value};

use crate::{
    McpCassette, McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession,
    mcp_tools_call_capability,
};

#[test]
fn cassette_records_and_replays_tool_call_without_callable() {
    let mut cx = test_cx();
    let symbol = Symbol::qualified("fixture", "echo");
    let calls = install_counter(&mut cx, symbol.clone());
    let capability = CapabilityName::new("fixture.echo.call");
    let session = McpSession::new("record", McpProfile::all())
        .with_cassette(McpCassette::new())
        .with_native_cards(vec![native_tool(
            symbol,
            "fixture.echo",
            capability.clone(),
        )])
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(capability);
    let mut router = McpRouter::new(session);
    let request = tools_call_request(
        "fixture.echo",
        vec![Expr::String("replay-value".to_owned())],
    );

    let recorded = expect_response(tools_call(&mut cx, &mut router, request.clone()));

    assert_eq!(calls.call_count(), 1);
    let cassette = router.session().cassette().unwrap();
    assert_eq!(cassette.entries().len(), 1);
    assert_eq!(cassette.audit()[0].method, "tools/call");
    assert_eq!(cassette.audit()[0].outcome, "ok");

    let replay = McpCassette::from_entries(cassette.entries().to_vec());
    let mut replay_router =
        McpRouter::new(McpSession::new("replay", McpProfile::all()).with_cassette(replay));
    let mut replay_cx = test_cx();
    let replayed = expect_response(tools_call(&mut replay_cx, &mut replay_router, request));

    assert_eq!(replayed, recorded);
    assert_eq!(
        replay_router.session().cassette().unwrap().audit()[0].outcome,
        "replay"
    );
}

#[test]
fn router_boundary_limits_are_structured_and_redacted() {
    let mut cx = test_cx();
    let mut rate = McpRouter::new(
        McpSession::new("rate", McpProfile::all())
            .with_cassette(McpCassette::new())
            .with_rate_limit(0),
    );
    let rate_error = expect_error(ping(&mut cx, &mut rate, "secret-rate"));
    assert_eq!(rate_error.error.code, RATE_LIMITED);
    assert_eq!(rate_error.error.message, "rate limited");
    assert!(!format!("{:?}", rate.session().cassette().unwrap().audit()).contains("secret-rate"));

    let mut deadline = McpRouter::new(
        McpSession::new("deadline", McpProfile::all())
            .with_cassette(McpCassette::new())
            .with_deadline_ms(0),
    );
    let deadline_error = expect_error(ping(&mut cx, &mut deadline, "secret-deadline"));
    assert_eq!(deadline_error.error.code, EXECUTION_ERROR);
    assert_eq!(deadline_error.error.message, "deadline exceeded");

    let mut active = McpRouter::new(
        McpSession::new("active", McpProfile::all())
            .with_cassette(McpCassette::new())
            .with_active_request_limit(1),
    );
    active
        .session_mut()
        .begin_request(&Expr::String("held".to_owned()));
    let active_error = expect_error(ping(&mut cx, &mut active, "secret-active"));
    assert_eq!(active_error.error.code, RATE_LIMITED);
    assert_eq!(active_error.error.message, "rate limited");
}

#[cfg(feature = "skill")]
#[test]
fn skill_resource_and_prompt_audit_records_are_redacted() {
    let mut cx = skill_cx();
    let fixture = bind_skill(&mut cx);
    let resource_capability = sim_lib_skill::skill_specific_call_capability("audit.resource");
    let prompt_capability = sim_lib_skill::skill_specific_call_capability("audit.prompt");
    cx.grant(resource_capability.clone());
    cx.grant(prompt_capability.clone());
    let session = McpSession::new("skill-audit", McpProfile::all())
        .with_cassette(McpCassette::new())
        .with_granted_capability(crate::mcp_resources_read_capability())
        .with_granted_capability(crate::mcp_prompts_get_capability())
        .with_granted_capability(resource_capability)
        .with_granted_capability(prompt_capability);
    let mut router = McpRouter::new(session);

    let _ = expect_response(resource_read(&mut cx, &mut router));
    let _ = expect_response(prompt_get(
        &mut cx,
        &mut router,
        vec![Expr::String("secret-prompt".to_owned())],
    ));

    assert_eq!(fixture.call_count(), 2);
    let audit = router.session().cassette().unwrap().audit();
    assert!(audit.iter().any(|entry| entry.method == "resources/read"));
    assert!(audit.iter().any(|entry| entry.method == "prompts/get"));
    assert!(!format!("{audit:?}").contains("secret-prompt"));
    assert!(
        !format!("{:?}", router.session().cassette().unwrap().entries()).contains("secret-prompt")
    );
}

#[cfg(feature = "sampling")]
#[test]
fn sampling_cassette_replays_without_host_call_and_audits() {
    use sim_lib_agent_runner_core::ModelRunner;

    let mut cx = test_cx();
    cx.grant(crate::mcp_sampling_capability());
    let cassette = Arc::new(Mutex::new(McpCassette::new()));
    let host = Arc::new(crate::FixtureSamplingHost::text("sampled"));
    let runner = crate::McpSamplingRunner::new(
        Symbol::qualified("mcp", "sampling"),
        "mcp/sample",
        host.clone(),
    )
    .with_cassette(cassette.clone());

    let response = runner.infer(&mut cx, model_request()).unwrap();

    assert_eq!(text_content(&response.content), Some("sampled"));
    assert_eq!(host.call_count(), 1);
    let entries = cassette.lock().unwrap().entries().to_vec();

    let replay = Arc::new(Mutex::new(McpCassette::from_entries(entries)));
    let unused = Arc::new(crate::FixtureSamplingHost::text("unused"));
    let replay_runner = crate::McpSamplingRunner::new(
        Symbol::qualified("mcp", "sampling"),
        "mcp/sample",
        unused.clone(),
    )
    .with_cassette(replay.clone());
    let replayed = replay_runner.infer(&mut cx, model_request()).unwrap();

    assert_eq!(text_content(&replayed.content), Some("sampled"));
    assert_eq!(unused.call_count(), 0);
    assert_eq!(replay.lock().unwrap().audit()[0].outcome, "replay");
}

#[cfg(feature = "sampling")]
fn model_request() -> sim_lib_agent_runner_core::ModelRequest {
    sim_lib_agent_runner_core::ModelRequest::new(Expr::String("hello".to_owned()), Vec::new())
}

#[cfg(feature = "sampling")]
fn text_content(content: &[Expr]) -> Option<&str> {
    content.iter().find_map(|part| {
        let Expr::Map(fields) = part else {
            return None;
        };
        fields.iter().find_map(|(key, value)| {
            if !matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "text") {
                return None;
            }
            match value {
                Expr::String(text) => Some(text.as_str()),
                _ => None,
            }
        })
    })
}

fn tools_call(cx: &mut Cx, router: &mut McpRouter, request: McpEnvelope) -> McpEnvelope {
    router.handle(cx, request).unwrap().unwrap()
}

fn tools_call_request(name: &str, arguments: Vec<Expr>) -> McpEnvelope {
    McpEnvelope::Request(McpRequest {
        id: Expr::String("tool-call".to_owned()),
        method: "tools/call".to_owned(),
        params: Expr::Map(vec![
            field("name", Expr::String(name.to_owned())),
            field("arguments", Expr::List(arguments)),
        ]),
    })
}

fn ping(cx: &mut Cx, router: &mut McpRouter, id: &str) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(id.to_owned()),
                method: "ping".to_owned(),
                params: Expr::Nil,
            }),
        )
        .unwrap()
        .unwrap()
}

#[cfg(feature = "skill")]
fn resource_read(cx: &mut Cx, router: &mut McpRouter) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String("resource-read".to_owned()),
                method: "resources/read".to_owned(),
                params: Expr::Map(vec![field(
                    "uri",
                    Expr::String("skill://audit.resource".to_owned()),
                )]),
            }),
        )
        .unwrap()
        .unwrap()
}

#[cfg(feature = "skill")]
fn prompt_get(cx: &mut Cx, router: &mut McpRouter, arguments: Vec<Expr>) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String("prompt-get".to_owned()),
                method: "prompts/get".to_owned(),
                params: Expr::Map(vec![
                    field("name", Expr::String("audit.prompt".to_owned())),
                    field("arguments", Expr::List(arguments)),
                ]),
            }),
        )
        .unwrap()
        .unwrap()
}

fn expect_response(envelope: McpEnvelope) -> Expr {
    let McpEnvelope::Response(McpResponse { result, .. }) = envelope else {
        panic!("expected response");
    };
    result
}

fn expect_error(envelope: McpEnvelope) -> McpErrorEnvelope {
    let McpEnvelope::Error(error) = envelope else {
        panic!("expected error");
    };
    error
}

fn native_tool(symbol: Symbol, name: &str, capability: CapabilityName) -> McpNativeCard {
    McpNativeCard::new(symbol, "Cassette fixture tool")
        .with_shapes(any_shape("args"), any_shape("result"))
        .with_capability(capability)
        .exported(McpExportFacet::tool().with_name(name.to_owned()))
}

fn install_counter(cx: &mut Cx, symbol: Symbol) -> Arc<CounterFunction> {
    let function = Arc::new(CounterFunction::default());
    cx.load_lib(&CounterLib {
        id: Symbol::qualified("mcp-cassette-test", symbol.to_string()),
        symbol,
        function: function.clone(),
    })
    .unwrap();
    function
}

#[derive(Default)]
struct CounterFunction {
    calls: AtomicUsize,
}

impl CounterFunction {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Object for CounterFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-cassette-counter>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for CounterFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for CounterFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        args.values()
            .first()
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| cx.factory().nil())
    }
}

struct CounterLib {
    id: Symbol,
    symbol: Symbol,
    function: Arc<CounterFunction>,
}

impl Lib for CounterLib {
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

#[cfg(feature = "skill")]
fn skill_cx() -> Cx {
    let mut cx = test_cx();
    sim_lib_skill::install_skill_lib(&mut cx).unwrap();
    cx.grant(sim_lib_skill::skill_call_capability());
    cx
}

#[cfg(feature = "skill")]
fn bind_skill(cx: &mut Cx) -> Arc<sim_lib_skill::FixtureTransport> {
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("audit"));
    fixture
        .insert("resource", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    fixture
        .insert("prompt", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    registry.install_transport(fixture.clone()).unwrap();
    for (id, role, operation) in [
        (
            "audit.resource",
            sim_lib_skill::SkillRole::Resource,
            "resource",
        ),
        ("audit.prompt", sim_lib_skill::SkillRole::Prompt, "prompt"),
    ] {
        let mut card = sim_lib_skill::SkillCard::fixture(sim_lib_skill::FixtureSkillSpec {
            id: id.to_owned(),
            symbol: Symbol::qualified("skill", id.to_owned()),
            title: "Audit Echo".to_owned(),
            description: "Audit fixture skill.".to_owned(),
            input_shape: any_shape("skill-args"),
            output_shape: any_shape("skill-result"),
            transport_id: "audit".to_owned(),
            operation: operation.to_owned(),
        });
        card.roles = vec![role];
        registry.bind_card(cx, card).unwrap();
    }
    fixture
}

fn test_cx() -> Cx {
    Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory))
}

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-cassette-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_value::build::entry as field;
