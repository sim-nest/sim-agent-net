use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use sim_codec_binary::BinaryCodecLib;
use sim_kernel::{
    CapabilityName, Consistency, Cx, DefaultFactory, EagerPolicy, Error, EvalFabric, EvalMode,
    EvalReply, EvalRequest, Expr, Result, Value, eval_remote_capability,
};
use sim_lib_server::{Connection, ServerAddress, connect_transport_site};
use sim_lib_stream_fabric::{
    ContentKey, EvalCassette, EvalCassetteLedger, LedgeredRelayFabric, RelayFabric,
};

#[derive(Default)]
struct MemoryLedger {
    entries: Mutex<Vec<(ContentKey, EvalReply)>>,
}

impl EvalCassetteLedger for MemoryLedger {
    fn append_eval_result(&self, key: &ContentKey, reply: &EvalReply) -> Result<()> {
        self.entries
            .lock()
            .unwrap()
            .push((key.clone(), reply.clone()));
        Ok(())
    }

    fn replay_eval_results(&self) -> Result<Vec<(ContentKey, EvalReply)>> {
        Ok(self.entries.lock().unwrap().clone())
    }
}

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn spawn() -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_sim-fabric-cadr-fixture"))
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("cadr fixture starts");
        Self { child }
    }

    fn port(&mut self) -> u16 {
        let stderr = self.child.stderr.take().expect("fixture stderr is piped");
        wait_for_cadr_ready(stderr)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

#[test]
fn content_addressed_realize_caches_after_first_cross_process_call() {
    let mut child = ChildGuard::spawn();
    let port = child.port();
    let mut cx = client_cx();
    let cassette = Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())));
    let connection = Arc::new(connection(&mut cx, port));
    let relay = RelayFabric::new(connection.clone(), vec![CapabilityName::new("network")]);
    let fabric = LedgeredRelayFabric::new(relay, cassette.clone());
    let request = request("cadr-probe", "network");

    let first = fabric.realize(&mut cx, request.clone()).unwrap();
    connection.close(&mut cx).unwrap();

    let second = fabric.realize(&mut cx, request.clone()).unwrap();
    assert_eq!(
        value_display(&mut cx, &second.value),
        value_display(&mut cx, &first.value),
        "cached reply must equal the live reply"
    );
    assert_eq!(cassette.len(), 1, "cassette must hold one reply");

    child.kill();
    let third = fabric.realize(&mut cx, request).unwrap();
    assert_eq!(
        value_display(&mut cx, &third.value),
        value_display(&mut cx, &first.value),
        "cassette must answer after node death"
    );
}

#[test]
fn capability_refusal_propagates_through_ledgered_layer() {
    let cassette = Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())));
    let fabric = LedgeredRelayFabric::new(
        RelayFabric::new(
            Arc::new(UnreachableFabric),
            vec![CapabilityName::new("network")],
        ),
        cassette.clone(),
    );
    let mut cx = client_cx();

    let Err(error) = fabric.realize(&mut cx, request("blocked", "fabric.denied")) else {
        panic!("capability refusal must propagate");
    };

    assert!(matches!(
        error,
        Error::CapabilityDenied { capability } if capability.as_str() == "fabric.denied"
    ));
    assert_eq!(cassette.len(), 0, "refusals must not be cached");
}

struct UnreachableFabric;

impl EvalFabric for UnreachableFabric {
    fn realize(&self, _cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
        Err(Error::Eval("unreachable fabric was called".to_owned()))
    }
}

fn connection(cx: &mut Cx, port: u16) -> Connection {
    let address = ServerAddress::Tcp {
        host: loopback_host(),
        port,
    };
    let (site, selected) = connect_transport_site(cx, address.clone(), codecs()).unwrap();
    Connection::new(address, selected, codecs(), site).unwrap()
}

fn wait_for_cadr_ready(stderr: impl std::io::Read) -> u16 {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut lines = BufReader::new(stderr).lines();
    loop {
        assert!(
            Instant::now() < deadline,
            "fixture did not report readiness"
        );
        let Some(line) = lines.next() else {
            panic!("fixture exited before reporting readiness");
        };
        let line = line.expect("fixture readiness line is readable");
        if let Some(port) = line.strip_prefix("CADR_READY ") {
            return port.parse().expect("fixture prints a numeric port");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn client_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).expect("binary codec loads");
    cx.grant_named("network");
    cx.grant(eval_remote_capability());
    cx
}

fn request(expr: &str, capability: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::String(expr.to_owned()),
        result_shape: None,
        required_capabilities: vec![CapabilityName::new(capability)],
        deadline: None,
        consistency: Consistency::RemoteOnly,
        mode: EvalMode::Eval,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

fn value_display(cx: &mut Cx, value: &Value) -> String {
    value.object().display(cx).unwrap()
}

fn codecs() -> Vec<sim_kernel::Symbol> {
    vec![sim_kernel::Symbol::qualified("codec", "binary")]
}

fn loopback_host() -> String {
    "127.0.0.1".to_owned()
}
