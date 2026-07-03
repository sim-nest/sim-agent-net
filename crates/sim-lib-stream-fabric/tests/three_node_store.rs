use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use sim_codec_binary::BinaryCodecLib;
use sim_kernel::{
    CapabilityName, Consistency, Cx, DefaultFactory, EagerPolicy, EvalFabric, EvalFabricRef,
    EvalMode, EvalReply, EvalRequest, Expr, Result, Symbol, Value, eval_remote_capability,
};
use sim_lib_server::{Connection, ServerAddress, connect_transport_site};
use sim_lib_stream_fabric::{
    ContentAddressedFabric, ContentKey, ContentPeer, EvalCassette, EvalCassetteLedger,
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
    label: &'static str,
    child: Child,
}

impl ChildGuard {
    fn spawn(label: &'static str, seeds: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sim-fabric-cadr-fixture"));
        command
            .args(["--serve-held", "--node", label, "--request-limit", "16"])
            .stderr(Stdio::piped())
            .stdout(Stdio::null());
        for (expr, value) in seeds {
            command.args(["--hold", expr, value]);
        }
        let child = command.spawn().expect("cadr fixture starts");
        Self { label, child }
    }

    fn label(&self) -> &'static str {
        self.label
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
fn three_process_store_resolves_by_content_id_and_replays_after_two_losses() {
    let mut node_a = ChildGuard::spawn("node-a", &[]);
    let mut node_b = ChildGuard::spawn("node-b", &[("shared", "from-b")]);
    let mut node_c = ChildGuard::spawn("node-c", &[]);
    let port_a = node_a.port();
    let port_b = node_b.port();
    let port_c = node_c.port();
    let mut cx = client_cx();
    let local = Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())));
    let fabric = ContentAddressedFabric::new(
        Symbol::new("client"),
        local.clone(),
        vec![
            ContentPeer::new(Symbol::new(node_a.label()), connect_serve(&mut cx, port_a)),
            ContentPeer::new(Symbol::new(node_b.label()), connect_serve(&mut cx, port_b)),
            ContentPeer::new(Symbol::new(node_c.label()), connect_serve(&mut cx, port_c)),
        ],
    );

    let shared = request("shared");
    let shared_key = ContentKey::from_request(&shared);
    let resolved = fabric.realize(&mut cx, shared.clone()).unwrap();
    assert_eq!(value_display(&mut cx, &resolved.value), "from-b");
    assert!(
        local.get(&shared_key).is_some(),
        "client becomes a holder after a TCP peer hit"
    );

    node_b.kill();
    node_c.kill();
    let replayed = fabric.realize(&mut cx, shared).unwrap();
    assert_eq!(value_display(&mut cx, &replayed.value), "from-b");

    let unknown = request("never-seen");
    let unknown_key = ContentKey::from_request(&unknown);
    let Err(error) = fabric.realize(&mut cx, unknown) else {
        panic!("unknown content must fail closed");
    };
    assert!(format!("{error}").contains("no holder"));
    assert!(
        local.get(&unknown_key).is_none(),
        "unknown content must not pollute the local cassette"
    );

    node_a.kill();
}

fn connect_serve(cx: &mut Cx, port: u16) -> EvalFabricRef {
    Arc::new(connection(cx, port))
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

fn request(expr: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::String(expr.to_owned()),
        result_shape: None,
        required_capabilities: vec![CapabilityName::new("network")],
        deadline: Some(Duration::from_millis(500)),
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

fn codecs() -> Vec<Symbol> {
    vec![Symbol::qualified("codec", "binary")]
}

fn loopback_host() -> String {
    "127.0.0.1".to_owned()
}
