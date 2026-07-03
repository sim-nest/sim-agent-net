use std::{
    env,
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
    CapabilityName, Consistency, Cx, DefaultFactory, EagerPolicy, Error, EvalFabric, EvalMode,
    EvalReply, EvalRequest, Expr, Result, Symbol,
};
use sim_lib_server::{
    EvalSite, ServerAddress, ServerFrame, ServerRuntime, TcpServerTransport, ThreadMode,
    eval_request_from_frame, server_frame_from_reply,
};
use sim_lib_stream_fabric::{ContentKey, ContentServeFabric, EvalCassette, EvalCassetteLedger};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("CADR_FIXTURE_ERROR {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let config = FixtureConfig::from_args(env::args().skip(1))?;
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
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).expect("binary codec loads");
    cx.grant_named("network");
    cx
}

fn codecs() -> Vec<Symbol> {
    vec![Symbol::qualified("codec", "binary")]
}

fn loopback_host() -> String {
    "127.0.0.1".to_owned()
}
