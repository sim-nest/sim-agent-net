use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use sim_codec_mcp::{
    CAPABILITY_DENIED, INVALID_PARAMS, McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse,
};
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, Cx, Error, Export, Lib, LibManifest, LibTarget,
    Linker, LoadCx, Object, ObjectCompat, Result, ShapeRef, Symbol, Value, Version,
};
use sim_shape::{AnyShape, ListShape, NumberValueShape, shape_value};

use crate::{
    McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession, call_symbol, install_mcp_lib,
    mcp_tools_call_capability,
};
use sim_kernel::Expr;

#[test]
fn tools_call_native_success_executes_exactly_once() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "echo");
    let counter = install_counter(&mut cx, symbol.clone(), CounterBehavior::ReturnFirst);
    let tool_capability = CapabilityName::new("fixture.native.call");
    let session = session_with_native_tool(
        symbol,
        "fixture.echo",
        tool_capability.clone(),
        number_args_shape(),
    )
    .with_granted_capability(mcp_tools_call_capability())
    .with_granted_capability(tool_capability);
    let mut router = McpRouter::new(session);

    let result = expect_response_result(tools_call(
        &mut cx,
        &mut router,
        "fixture.echo",
        vec![number_expr(7)],
    ));

    assert_eq!(counter.call_count(), 1);
    assert_eq!(is_error(&result), Some(false));
    assert_eq!(single_json_content(&result), Some(&number_expr(7)));
}

#[test]
fn tools_call_capability_denial_does_not_execute_callable() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "denied");
    let counter = install_counter(&mut cx, symbol.clone(), CounterBehavior::ReturnFirst);
    let tool_capability = CapabilityName::new("fixture.denied.call");
    let session = session_with_native_tool(
        symbol,
        "fixture.denied",
        tool_capability,
        any_args_shape("denied-args"),
    )
    .with_granted_capability(mcp_tools_call_capability());
    let mut router = McpRouter::new(session);

    let error = expect_error(tools_call(
        &mut cx,
        &mut router,
        "fixture.denied",
        vec![Expr::String("payload".to_owned())],
    ));

    assert_eq!(counter.call_count(), 0);
    assert_eq!(error.error.code, CAPABILITY_DENIED);
}

#[test]
fn tools_call_invalid_arguments_do_not_execute_callable() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "typed");
    let counter = install_counter(&mut cx, symbol.clone(), CounterBehavior::ReturnFirst);
    let tool_capability = CapabilityName::new("fixture.typed.call");
    let session = session_with_native_tool(
        symbol,
        "fixture.typed",
        tool_capability.clone(),
        number_args_shape(),
    )
    .with_granted_capability(mcp_tools_call_capability())
    .with_granted_capability(tool_capability);
    let mut router = McpRouter::new(session);

    let error = expect_error(tools_call(
        &mut cx,
        &mut router,
        "fixture.typed",
        vec![Expr::String("not-a-number".to_owned())],
    ));

    assert_eq!(counter.call_count(), 0);
    assert_eq!(error.error.code, INVALID_PARAMS);
}

#[test]
fn tools_call_execution_errors_become_tool_results() {
    let mut cx = cx();
    let symbol = Symbol::qualified("fixture", "fails");
    let counter = install_counter(&mut cx, symbol.clone(), CounterBehavior::Fail);
    let tool_capability = CapabilityName::new("fixture.fails.call");
    let session = session_with_native_tool(
        symbol,
        "fixture.fails",
        tool_capability.clone(),
        any_args_shape("fails-args"),
    )
    .with_granted_capability(mcp_tools_call_capability())
    .with_granted_capability(tool_capability);
    let mut router = McpRouter::new(session);

    let result = expect_response_result(tools_call(
        &mut cx,
        &mut router,
        "fixture.fails",
        vec![Expr::String("payload".to_owned())],
    ));

    assert_eq!(counter.call_count(), 1);
    assert_eq!(is_error(&result), Some(true));
    assert!(single_text_content(&result).is_some_and(|text| text.contains("fixture failure")));
}

#[test]
fn tools_call_mcp_call_function_is_registered() {
    let mut cx = cx();
    install_mcp_lib(&mut cx).unwrap();
    let params = call_params("missing.tool", Vec::new());
    let value = cx.factory().expr(params).unwrap();

    let err = cx
        .call_function(&call_symbol(), Args::new(vec![value]))
        .unwrap_err();

    assert!(format!("{err}").contains("unknown MCP tool missing.tool"));
}

#[cfg(feature = "skill")]
#[test]
fn tools_call_skill_backed_tool_uses_bound_skill_callable_once() {
    let mut cx = skill_cx();
    cx.grant(sim_lib_skill::skill_call_capability());
    cx.grant(sim_lib_skill::skill_specific_call_capability("skill.echo"));
    let fixture = bind_skill(&mut cx, skill_card("skill.echo"));
    let session = McpSession::new("skill-call", McpProfile::all())
        .with_granted_capability(mcp_tools_call_capability())
        .with_granted_capability(sim_lib_skill::skill_specific_call_capability("skill.echo"));
    let mut router = McpRouter::new(session);

    let result = expect_response_result(tools_call(
        &mut cx,
        &mut router,
        "skill.echo",
        vec![Expr::String("hello".to_owned())],
    ));

    assert_eq!(fixture.call_count(), 1);
    assert_eq!(is_error(&result), Some(false));
    assert_eq!(
        single_json_content(&result),
        Some(&Expr::List(vec![Expr::String("hello".to_owned())]))
    );
}

fn tools_call(
    cx: &mut Cx,
    router: &mut McpRouter,
    name: &str,
    arguments: Vec<Expr>,
) -> McpEnvelope {
    router
        .handle(
            cx,
            McpEnvelope::Request(McpRequest {
                id: Expr::String(format!("call-{name}")),
                method: "tools/call".to_owned(),
                params: call_params(name, arguments),
            }),
        )
        .unwrap()
        .unwrap()
}

fn call_params(name: &str, arguments: Vec<Expr>) -> Expr {
    Expr::Map(vec![
        field("name", Expr::String(name.to_owned())),
        field("arguments", Expr::List(arguments)),
    ])
}

fn session_with_native_tool(
    symbol: Symbol,
    name: &str,
    capability: CapabilityName,
    input_shape: ShapeRef,
) -> McpSession {
    McpSession::new("tools-call", McpProfile::all()).with_native_cards(vec![
        McpNativeCard::new(symbol, "Fixture MCP native tool")
            .with_shapes(input_shape, any_args_shape("result"))
            .with_capability(capability)
            .exported(McpExportFacet::tool().with_name(name.to_owned())),
    ])
}

fn install_counter(cx: &mut Cx, symbol: Symbol, behavior: CounterBehavior) -> Arc<CounterFunction> {
    let function = Arc::new(CounterFunction::new(behavior));
    cx.load_lib(&CounterLib {
        id: Symbol::qualified("mcp-test", symbol.to_string()),
        symbol,
        function: function.clone(),
    })
    .unwrap();
    function
}

#[derive(Clone, Copy)]
enum CounterBehavior {
    ReturnFirst,
    Fail,
}

struct CounterFunction {
    calls: AtomicUsize,
    behavior: CounterBehavior,
}

impl CounterFunction {
    fn new(behavior: CounterBehavior) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            behavior,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Object for CounterFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-test-counter>".to_owned())
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
        match self.behavior {
            CounterBehavior::ReturnFirst => args
                .values()
                .first()
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| cx.factory().nil()),
            CounterBehavior::Fail => Err(Error::Eval("fixture failure".to_owned())),
        }
    }

    fn browse_args_shape(&self, _cx: &mut Cx) -> Result<Option<ShapeRef>> {
        Ok(Some(any_args_shape("counter-args")))
    }

    fn browse_result_shape(&self, _cx: &mut Cx) -> Result<Option<ShapeRef>> {
        Ok(Some(any_args_shape("counter-result")))
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

fn expect_response_result(envelope: McpEnvelope) -> Expr {
    let McpEnvelope::Response(McpResponse { result, .. }) = envelope else {
        panic!("expected MCP response envelope");
    };
    result
}

fn expect_error(envelope: McpEnvelope) -> McpErrorEnvelope {
    let McpEnvelope::Error(error) = envelope else {
        panic!("expected MCP error envelope");
    };
    error
}

fn is_error(result: &Expr) -> Option<bool> {
    match field_value(result, "isError") {
        Some(Expr::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn single_json_content(result: &Expr) -> Option<&Expr> {
    single_content(result).and_then(|content| field_value(content, "json"))
}

fn single_text_content(result: &Expr) -> Option<&str> {
    single_content(result)
        .and_then(|content| field_value(content, "text"))
        .and_then(|text| match text {
            Expr::String(text) => Some(text.as_str()),
            _ => None,
        })
}

fn single_content(result: &Expr) -> Option<&Expr> {
    match field_value(result, "content") {
        Some(Expr::List(items)) if items.len() == 1 => items.first(),
        _ => None,
    }
}

fn field_value<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(fields) = expr else {
        return None;
    };
    fields.iter().find_map(|(key, value)| {
        let key = match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => return None,
        };
        (key == name).then_some(value)
    })
}

fn number_expr(value: u32) -> Expr {
    Expr::Number(sim_kernel::NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn number_args_shape() -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-test", "number-args"),
        Arc::new(ListShape::new(vec![Arc::new(NumberValueShape)])),
    )
}

fn any_args_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("mcp-test", name.to_owned()),
        Arc::new(AnyShape),
    )
}

use sim_kernel::testing::eager_cx as cx;

use sim_value::build::entry as field;

#[cfg(feature = "skill")]
fn skill_cx() -> Cx {
    let mut cx = cx();
    sim_lib_skill::install_skill_lib(&mut cx).unwrap();
    cx
}

#[cfg(feature = "skill")]
fn skill_card(id: &str) -> sim_lib_skill::SkillCard {
    sim_lib_skill::SkillCard::fixture(sim_lib_skill::FixtureSkillSpec {
        id: id.to_owned(),
        symbol: Symbol::qualified("skill", id.to_owned()),
        title: "Fixture Skill".to_owned(),
        description: "A skill projected through tools/call.".to_owned(),
        input_shape: any_args_shape("skill-args"),
        output_shape: any_args_shape("skill-result"),
        transport_id: "tools-call-transport".to_owned(),
        operation: "echo".to_owned(),
    })
}

#[cfg(feature = "skill")]
fn bind_skill(cx: &mut Cx, card: sim_lib_skill::SkillCard) -> Arc<sim_lib_skill::FixtureTransport> {
    let registry = sim_lib_skill::skill_registry(cx).unwrap();
    let fixture = Arc::new(sim_lib_skill::FixtureTransport::new("tools-call-transport"));
    fixture
        .insert("echo", sim_lib_skill::FixtureBehavior::EchoArgs)
        .unwrap();
    registry.install_transport(fixture.clone()).unwrap();
    registry.bind_card(cx, card).unwrap();
    fixture
}
