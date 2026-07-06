use std::{
    ffi::OsString,
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use sim_codec_binary::BinaryCodecLib;
use sim_kernel::{
    AbiVersion, Args, Callable, CapabilityName, CodecId, Consistency, Cx, DefaultFactory,
    EagerPolicy, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Export, Expr, Lib,
    LibManifest, LibTarget, Linker, LoadCx, Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_lib_server::{
    EvalSite, ServerAddress, ServerFrame, ServerRuntime, TcpServerTransport, ThreadMode,
    eval_request_from_frame, server_frame_from_reply,
};
use sim_lib_stream_fabric::{ContentKey, ContentServeFabric, EvalCassette, EvalCassetteLedger};
use sim_run_core::{Bootloader, cli_main_entrypoint_symbol};

/// The verb the bootloader dispatches to run the fixture (`sim-fabric-cadr-fixture`).
const CADR_FIXTURE_VERB: &str = "sim-fabric-cadr-fixture";

fn main() -> ExitCode {
    // Boot through sim-run like every other product binary: the bootloader dispatches
    // the `sim-fabric-cadr-fixture` verb into the fixture entrypoint. The fixture's
    // ServerRuntime owns its own serving `Cx` (a distinct eval surface), which is why
    // `cx()` below carries a `bin-boot-exempt` annotation.
    let mut args: Vec<OsString> = [
        "sim-fabric-cadr-fixture",
        "--codec",
        "binary",
        CADR_FIXTURE_VERB,
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.extend(std::env::args_os().skip(1));
    match cadr_fixture_bootloader().run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("CADR_FIXTURE_ERROR {error}");
            ExitCode::FAILURE
        }
    }
}

/// A [`Bootloader`] pre-configured to run the CADR fixture: the `codec/binary` boot
/// codec plus the fixture verb.
fn cadr_fixture_bootloader() -> Bootloader {
    Bootloader::standard()
        .host_lib("codec/binary", || Box::new(BinaryCodecLib::new(CodecId(1))))
        .host_verb(CADR_FIXTURE_VERB, "lib/cadr-fixture", || {
            Box::new(CadrFixtureLib)
        })
        .with_capability(CapabilityName::new("network"))
}

/// Loadable library exporting the fixture `cli/main/sim-fabric-cadr-fixture` entrypoint.
struct CadrFixtureLib;

impl Lib for CadrFixtureLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("lib", "cadr-fixture"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Function {
                symbol: cli_main_entrypoint_symbol(CADR_FIXTURE_VERB),
                function_id: None,
            }],
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        linker.function_value(
            cli_main_entrypoint_symbol(CADR_FIXTURE_VERB),
            cx.factory().opaque(Arc::new(CadrFixtureEntrypoint))?,
        )?;
        Ok(())
    }
}

#[derive(Clone)]
struct CadrFixtureEntrypoint;

impl Object for CadrFixtureEntrypoint {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("cli/main/sim-fabric-cadr-fixture".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for CadrFixtureEntrypoint {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for CadrFixtureEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        // Parse the fixture flags from the boot envelope (skipping the verb token),
        // then run the fixture in its own ServerRuntime cx.
        let config = match args.values().first() {
            Some(envelope) => {
                let payload = envelope_args(cx, envelope)?;
                FixtureConfig::from_args(payload.into_iter().skip(1))?
            }
            None => FixtureConfig::from_args(std::iter::empty())?,
        };
        run_fixture(config)?;
        cx.factory().bool(true)
    }
}

/// Extracts the payload argument list from the CLI boot envelope.
fn envelope_args(cx: &mut Cx, envelope: &Value) -> Result<Vec<String>> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    let value = table.get(cx, Symbol::new("args"))?;
    let Some(list) = value.object().as_list() else {
        return Err(Error::Eval(
            "CLI envelope field args is not a list".to_owned(),
        ));
    };
    list.to_vec(cx, Some(64))?
        .into_iter()
        .map(|value| match value.object().as_expr(cx)? {
            Expr::String(text) => Ok(text),
            other => Err(Error::Eval(format!(
                "CLI payload argument is not a string: {other:?}"
            ))),
        })
        .collect()
}

fn run_fixture(config: FixtureConfig) -> Result<()> {
    let transport = Arc::new(TcpServerTransport::bind(ServerAddress::Tcp {
        host: loopback_host(),
        port: 0,
    })?);
    let port = transport.local_port()?;
    let address = ServerAddress::Tcp {
        host: loopback_host(),
        port,
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let site: Arc<dyn EvalSite> = match config.mode {
        FixtureMode::Echo => Arc::new(EchoSite::new(address, requests.clone())),
        FixtureMode::ServeHeld { node, seeds } => {
            eprintln!("CADR_NODE {node}");
            Arc::new(HeldServeSite::new(address, seeds, requests.clone())?)
        }
    };
    let runtime = Arc::new(ServerRuntime::new(transport, cx(), ThreadMode::Main, 8));
    eprintln!("CADR_READY {port}");

    serve_bounded(runtime, site, requests, config.request_limit)
}

fn serve_bounded(
    runtime: Arc<ServerRuntime>,
    site: Arc<dyn EvalSite>,
    requests: Arc<AtomicUsize>,
    request_limit: usize,
) -> Result<()> {
    while requests.load(Ordering::SeqCst) < request_limit {
        let Some(mut connection) = runtime.accept_timeout(Duration::from_millis(25))? else {
            thread::sleep(Duration::from_millis(25));
            continue;
        };
        connection.serve_connection(&runtime, &site)?;
    }
    runtime.join_worker_threads()
}

#[derive(Clone)]
struct EchoSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
    requests: Arc<AtomicUsize>,
}

impl EchoSite {
    fn new(address: ServerAddress, requests: Arc<AtomicUsize>) -> Self {
        Self {
            address,
            codecs: codecs(),
            requests,
        }
    }
}

impl EvalSite for EchoSite {
    fn site_kind(&self) -> &'static str {
        "cadr-echo"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let request = eval_request_from_frame(cx, &frame)?;
        for capability in &request.required_capabilities {
            cx.require(capability)?;
        }
        let value = cx.factory().expr(request_expr(request)?)?;
        let mut reply = server_frame_from_reply(
            cx,
            &frame.codec,
            EvalReply {
                value,
                diagnostics: Vec::new(),
                trace: None,
            },
            frame.envelope.consistency,
        )?;
        reply.correlate = frame.msg_id;
        self.requests.fetch_add(1, Ordering::SeqCst);
        Ok(reply)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct HeldServeSite {
    address: ServerAddress,
    codecs: Vec<Symbol>,
    serve: ContentServeFabric,
    requests: Arc<AtomicUsize>,
}

impl HeldServeSite {
    fn new(address: ServerAddress, seeds: Vec<Seed>, requests: Arc<AtomicUsize>) -> Result<Self> {
        let cassette = Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())));
        let seed_cx = cx();
        for seed in seeds {
            let request = seed_request(&seed.expr);
            let reply = EvalReply {
                value: seed_cx.factory().string(seed.value)?,
                diagnostics: Vec::new(),
                trace: None,
            };
            cassette.record(ContentKey::from_request(&request), reply)?;
        }

        Ok(Self {
            address,
            codecs: codecs(),
            serve: ContentServeFabric::new(cassette),
            requests,
        })
    }
}

impl EvalSite for HeldServeSite {
    fn site_kind(&self) -> &'static str {
        "content-serve"
    }

    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn codecs(&self) -> &[Symbol] {
        &self.codecs
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        let request = eval_request_from_frame(cx, &frame)?;
        self.requests.fetch_add(1, Ordering::SeqCst);
        for capability in &request.required_capabilities {
            cx.require(capability)?;
        }
        let served = self.serve.realize(cx, request)?;
        let mut reply =
            server_frame_from_reply(cx, &frame.codec, served, frame.envelope.consistency)?;
        reply.correlate = frame.msg_id;
        Ok(reply)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Default)]
struct MemoryLedger {
    entries: Mutex<Vec<(ContentKey, EvalReply)>>,
}

impl EvalCassetteLedger for MemoryLedger {
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()> {
        self.entries
            .lock()
            .map_err(|_| Error::Eval("fixture ledger mutex poisoned".to_owned()))?
            .push((key.clone(), reply.clone()));
        Ok(())
    }

    fn replay_eval_results(&self) -> Result<Vec<(ContentKey, EvalReply)>> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| Error::Eval("fixture ledger mutex poisoned".to_owned()))?
            .clone())
    }
}

struct FixtureConfig {
    mode: FixtureMode,
    request_limit: usize,
}

enum FixtureMode {
    Echo,
    ServeHeld { node: String, seeds: Vec<Seed> },
}

struct Seed {
    expr: String,
    value: String,
}

impl FixtureConfig {
    fn from_args(args: impl Iterator<Item = String>) -> Result<Self> {
        let mut mode = FixtureMode::Echo;
        let mut request_limit = 3usize;
        let mut args = args.peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--serve-held" => {
                    mode = FixtureMode::ServeHeld {
                        node: "node".to_owned(),
                        seeds: Vec::new(),
                    };
                }
                "--node" => {
                    let node = next_arg(&mut args, "--node")?;
                    match &mut mode {
                        FixtureMode::Echo => {
                            mode = FixtureMode::ServeHeld {
                                node,
                                seeds: Vec::new(),
                            };
                        }
                        FixtureMode::ServeHeld { node: slot, .. } => *slot = node,
                    }
                }
                "--hold" => {
                    let expr = next_arg(&mut args, "--hold expr")?;
                    let value = next_arg(&mut args, "--hold value")?;
                    match &mut mode {
                        FixtureMode::Echo => {
                            mode = FixtureMode::ServeHeld {
                                node: "node".to_owned(),
                                seeds: vec![Seed { expr, value }],
                            };
                        }
                        FixtureMode::ServeHeld { seeds, .. } => {
                            seeds.push(Seed { expr, value });
                        }
                    }
                }
                "--request-limit" => {
                    request_limit = next_arg(&mut args, "--request-limit")?
                        .parse()
                        .map_err(|_| Error::Eval("--request-limit must be a usize".to_owned()))?;
                }
                other => {
                    return Err(Error::Eval(format!("unknown fixture argument {other}")));
                }
            }
        }

        Ok(Self {
            mode,
            request_limit,
        })
    }
}

fn next_arg(
    args: &mut std::iter::Peekable<impl Iterator<Item = String>>,
    flag: &str,
) -> Result<String> {
    args.next()
        .ok_or_else(|| Error::Eval(format!("{flag} requires a value")))
}

fn request_expr(request: EvalRequest) -> Result<Expr> {
    if request.stream {
        return Err(Error::Eval(
            "cadr fixture accepts non-stream requests".to_owned(),
        ));
    }
    Ok(request.expr)
}

fn seed_request(expr: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::String(expr.to_owned()),
        result_shape: None,
        required_capabilities: vec![CapabilityName::new("network")],
        deadline: None,
        consistency: Consistency::RemoteOnly,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn cx() -> Cx {
    // The fixture's ServerRuntime owns its serving Cx -- a distinct eval surface
    // answering remote eval requests on worker threads, not the binary's boot runtime
    // (that goes through sim_run_core::Bootloader in `main`).
    let (mut cx, seat) = Cx::new_seated(Arc::new(EagerPolicy), Arc::new(DefaultFactory)); // bin-boot-exempt
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).expect("binary codec loads");
    seat.grant_named(&mut cx, "network");
    cx
}

fn codecs() -> Vec<Symbol> {
    vec![Symbol::qualified("codec", "binary")]
}

fn loopback_host() -> String {
    "127.0.0.1".to_owned()
}
