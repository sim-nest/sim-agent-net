use std::io::{BufReader, Cursor};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::stdio::{StdioOptions, run_stdio};
use crate::{
    McpExportFacet, McpNativeCard, McpProfile, McpRouter, McpSession, mcp_tools_call_capability,
};
use sim_codec::encode_with_codec;
use sim_codec_mcp::{McpCodecLib, McpEnvelope, McpRequest, envelope_to_expr};
use sim_kernel::{
    AbiVersion, Args, Callable, Cx, DefaultFactory, EagerPolicy, EncodeOptions, Error, Export,
    Expr, Lib, LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value,
    Version,
};

#[test]
fn stdio_ping_has_protocol_only_stdout() {
    let mut cx = test_cx();
    let mut router = McpRouter::fixture();
    let (stdout, stderr) = run_loop(
        &mut cx,
        &mut router,
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
    );

    assert_eq!(stderr, "");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(r#""jsonrpc":"2.0""#));
    assert!(stdout.contains(r#""id":1"#));
    assert!(stdout.contains(r#""result""#));
    assert!(stdout.contains(r#""entries":[]"#));
}

#[test]
fn stdio_malformed_input_returns_parse_error() {
    let mut cx = test_cx();
    let mut router = McpRouter::fixture();
    let (stdout, stderr) = run_loop(&mut cx, &mut router, "{bad json");

    assert_eq!(stderr, "");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(r#""code":-32700"#));
    assert!(stdout.contains(r#""message":"parse error""#));
}

#[test]
fn stdio_forged_read_eval_expr_tag_returns_parse_error() {
    let mut cx = test_cx();
    let mut router = McpRouter::fixture();
    let (stdout, stderr) = run_loop(
        &mut cx,
        &mut router,
        r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{"$expr":"read-eval","value":1}}"#,
    );

    assert_eq!(stderr, "");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.contains(r#""code":-32700"#));
    assert!(stdout.contains(r#""message":"parse error""#));
}

#[test]
fn stdio_oversized_frame_is_rejected() {
    // A frame past the per-line cap must error out rather than grow memory
    // unbounded (regression for the uncapped `reader.lines()` reader).
    let mut cx = test_cx();
    let mut router = McpRouter::fixture();
    let oversized = "x".repeat(128 * 1024);
    let reader = BufReader::new(Cursor::new(oversized.into_bytes()));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let err = run_stdio(
        &mut cx,
        &mut router,
        reader,
        &mut stdout,
        &mut stderr,
        StdioOptions { log_stderr: false },
    )
    .expect_err("an oversized stdio frame must error, not accumulate");
    assert!(matches!(err, Error::HostError(message) if message.contains("exceeds")));
}

#[test]
fn stdio_eof_exits_cleanly() {
    let mut cx = test_cx();
    let mut router = McpRouter::fixture();
    let (stdout, stderr) = run_loop(&mut cx, &mut router, "");

    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn stdio_initialize_tools_call_and_shutdown_work() {
    let mut cx = test_cx();
    let symbol = Symbol::qualified("fixture", "echo");
    let counter = install_counter(&mut cx, symbol.clone());
    let session = McpSession::new("stdio-test", McpProfile::all())
        .with_granted_capability(mcp_tools_call_capability())
        .with_native_cards(vec![
            McpNativeCard::new(symbol, "Fixture echo")
                .exported(McpExportFacet::tool().with_name("fixture.echo")),
        ]);
    let mut router = McpRouter::new(session);
    let input = [
        request_frame(&mut cx, 1, "initialize", Expr::Nil),
        request_frame(&mut cx, 2, "tools/list", Expr::Nil),
        request_frame(&mut cx, 3, "tools/call", tools_call_params()),
        request_frame(&mut cx, 4, "shutdown", Expr::Nil),
    ]
    .join("\n");
    let (stdout, stderr) = run_loop(&mut cx, &mut router, &input);

    assert_eq!(stderr, "");
    assert_eq!(stdout.lines().count(), 4);
    assert!(stdout.contains(r#""serverInfo""#));
    assert!(stdout.contains(r#""tools""#));
    assert!(stdout.contains(r#""isError""#));
    assert!(stdout.contains(r#""value":false"#));
    assert_eq!(counter.call_count(), 1);
}

fn run_loop(cx: &mut Cx, router: &mut McpRouter, input: &str) -> (String, String) {
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    run_stdio(
        cx,
        router,
        reader,
        &mut stdout,
        &mut stderr,
        StdioOptions { log_stderr: false },
    )
    .unwrap();
    (
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn test_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let codec = McpCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&codec).unwrap();
    cx
}

fn request_frame(cx: &mut Cx, id: i64, method: &str, params: Expr) -> String {
    let expr = envelope_to_expr(&McpEnvelope::Request(McpRequest {
        id: Expr::Number(sim_kernel::NumberLiteral {
            domain: Symbol::qualified("numbers", "i64"),
            canonical: id.to_string(),
        }),
        method: method.to_owned(),
        params,
    }));
    encode_with_codec(
        cx,
        &Symbol::qualified("codec", "mcp"),
        &expr,
        EncodeOptions::default(),
    )
    .unwrap()
    .into_text()
    .unwrap()
}

fn tools_call_params() -> Expr {
    Expr::Map(vec![
        field("name", Expr::String("fixture.echo".to_owned())),
        field("arguments", Expr::List(vec![Expr::String("ok".to_owned())])),
    ])
}

use sim_value::build::entry as field;

fn install_counter(cx: &mut Cx, symbol: Symbol) -> Arc<CounterFunction> {
    let function = Arc::new(CounterFunction::new());
    cx.load_lib(&CounterLib {
        id: Symbol::qualified("mcp-stdio-test", symbol.to_string()),
        symbol,
        function: function.clone(),
    })
    .unwrap();
    function
}

struct CounterFunction {
    calls: AtomicUsize,
}

impl CounterFunction {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Object for CounterFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<mcp-stdio-counter>".to_owned())
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
