use std::sync::{Arc, Mutex};

use sim_codec_bridge::{
    BridgeBook, BridgeHeader, BridgePacket, BridgePart, BridgeProvenance, encode_bridge_text,
    stamp_packet_cid,
};
use sim_kernel::{
    CapabilityName, Consistency, Cx, DefaultFactory, EagerPolicy, Error, EvalFabric, EvalMode,
    EvalReply, EvalRequest, Expr, Result, Symbol,
};
use sim_lib_agent_runner_core::ModelResponse;
use sim_lib_stream_fabric::{ContentKey, EvalCassette, EvalCassetteLedger, LedgeredRelayFabric};
use sim_value::build::entry;

use crate::{
    bridge_request_content_key, bridge_rx_response, bridge_tx, effective_caps, run_bridge, rx_check,
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

struct CountingFabric {
    response: ModelResponse,
    calls: Arc<Mutex<usize>>,
    required: Option<CapabilityName>,
}

impl CountingFabric {
    fn new(response: ModelResponse, calls: Arc<Mutex<usize>>) -> Self {
        Self {
            response,
            calls,
            required: None,
        }
    }

    fn requiring(mut self, capability: CapabilityName) -> Self {
        self.required = Some(capability);
        self
    }
}

impl EvalFabric for CountingFabric {
    fn realize(&self, cx: &mut Cx, _request: EvalRequest) -> Result<EvalReply> {
        if let Some(capability) = &self.required {
            cx.require(capability)?;
        }
        *self.calls.lock().unwrap() += 1;
        Ok(EvalReply {
            value: cx.factory().expr(Expr::from(self.response.clone()))?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

fn cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    cx.grant(CapabilityName::new("ai/run"));
    cx.grant(CapabilityName::new("bridge/given.materialize"));
    cx
}

fn request_packet(return_shape: Expr, ceiling: Vec<Symbol>) -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec!["model:drafter".to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("T1"),
            output: Symbol::new("O1"),
            ceiling,
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("T1"),
                kind: Symbol::qualified("bridge", "Frame"),
                payload: Expr::Map(vec![entry(
                    "frame",
                    Expr::Symbol(Symbol::qualified("bridge", "proposal")),
                )]),
            },
            BridgePart {
                id: Symbol::new("O1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(Symbol::qualified("codec", "bridge"))),
                    entry("shape", return_shape),
                ]),
            },
        ],
        warrant: None,
    }
}

fn reply_packet(parent: &BridgePacket, output: Expr) -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("reply"),
            from: "model:drafter".to_owned(),
            to: vec!["sim".to_owned()],
            role: Symbol::new("implementer"),
            parents: vec![parent.header.cid.clone().unwrap()],
            task: Symbol::new("T2"),
            output: Symbol::new("O2"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("T2"),
                kind: Symbol::qualified("bridge", "Frame"),
                payload: Expr::Map(vec![entry(
                    "frame",
                    Expr::Symbol(Symbol::qualified("bridge", "answer")),
                )]),
            },
            BridgePart {
                id: Symbol::new("O2"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: output,
            },
        ],
        warrant: None,
    }
}

fn response_for(packet: &BridgePacket) -> ModelResponse {
    let text = encode_bridge_text(packet, &BridgeBook::standard()).unwrap();
    ModelResponse::new(
        Symbol::qualified("runner", "fixture"),
        "fixture",
        vec![text_content("progress".to_owned()), text_content(text)],
        Symbol::new("stop"),
    )
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        entry("type", Expr::Symbol(Symbol::new("text"))),
        entry("text", Expr::String(text)),
    ])
}

fn eval_request(task: &str) -> EvalRequest {
    EvalRequest {
        expr: Expr::String(task.to_owned()),
        result_shape: None,
        required_capabilities: Vec::new(),
        deadline: None,
        consistency: Consistency::default(),
        mode: EvalMode::default(),
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    }
}

#[test]
fn tx_refuses_packet_its_own_rx_would_bounce() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let mut packet = request_packet(
        Expr::Symbol(Symbol::qualified("core", "Any")),
        vec![Symbol::qualified("ai", "run")],
    );
    packet.body[0].payload = Expr::String("not a frame record".to_owned());
    let stamped = stamp_packet_cid(&packet).unwrap();
    let report = rx_check(&mut cx, &book, &stamped, None).unwrap();
    let err = match bridge_tx(&mut cx, &book, &packet) {
        Ok(_) => panic!("wrong-shape packet unexpectedly passed TX"),
        Err(err) => err,
    };

    assert!(!report.accepted());
    assert!(report.obligations[0].path.contains("T1"));
    assert!(err.to_string().contains(&report.obligations[0].path));
}

#[test]
fn identical_packets_share_content_key() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let packet = stamp_packet_cid(&request_packet(
        Expr::Symbol(Symbol::qualified("core", "Any")),
        vec![Symbol::qualified("ai", "run")],
    ))
    .unwrap();

    let left = bridge_request_content_key(&mut cx, &book, &packet).unwrap();
    let right = bridge_request_content_key(&mut cx, &book, &packet).unwrap();

    assert_eq!(left, right);
}

#[test]
fn reply_failing_parent_return_contract_rejects() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let parent = stamp_packet_cid(&request_packet(
        Expr::Symbol(Symbol::qualified("core", "String")),
        vec![Symbol::qualified("ai", "run")],
    ))
    .unwrap();
    let reply = stamp_packet_cid(&reply_packet(&parent, Expr::Bool(false))).unwrap();
    let report = rx_check(&mut cx, &book, &reply, Some(&parent)).unwrap();

    assert!(!report.accepted());
    assert!(
        report
            .obligations
            .iter()
            .any(|obligation| obligation.expected == "parent Return contract")
    );
}

#[test]
fn call_above_ceiling_fails_closed() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let parent = stamp_packet_cid(&request_packet(
        Expr::Symbol(Symbol::qualified("core", "String")),
        Vec::new(),
    ))
    .unwrap();
    let reply = stamp_packet_cid(&reply_packet(&parent, Expr::String("ok".to_owned()))).unwrap();
    let calls = Arc::new(Mutex::new(0));
    let fabric = CountingFabric::new(response_for(&reply), calls.clone())
        .requiring(CapabilityName::new("ai/run"));
    let err = run_bridge(&mut cx, &fabric, &book, parent).unwrap_err();

    assert!(
        matches!(err, Error::CapabilityDenied { capability } if capability.as_str() == "ai/run")
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn cassette_hit_skips_live_runner() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let parent = stamp_packet_cid(&request_packet(
        Expr::Symbol(Symbol::qualified("core", "String")),
        vec![Symbol::qualified("ai", "run")],
    ))
    .unwrap();
    let reply = stamp_packet_cid(&reply_packet(&parent, Expr::String("ok".to_owned()))).unwrap();
    let calls = Arc::new(Mutex::new(0));
    let fabric = CountingFabric::new(response_for(&reply), calls.clone());
    let cassette = Arc::new(EvalCassette::new(Arc::new(MemoryLedger::default())));
    let cached = LedgeredRelayFabric::new(fabric, cassette);

    run_bridge(&mut cx, &cached, &book, parent.clone()).unwrap();
    run_bridge(&mut cx, &cached, &book, parent).unwrap();

    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn receive_uses_terminal_content_item() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let parent = stamp_packet_cid(&request_packet(
        Expr::Symbol(Symbol::qualified("core", "String")),
        vec![Symbol::qualified("ai", "run")],
    ))
    .unwrap();
    let reply = stamp_packet_cid(&reply_packet(&parent, Expr::String("ok".to_owned()))).unwrap();
    let response = response_for(&reply);
    let (decoded, report) = bridge_rx_response(&mut cx, &book, &response, Some(&parent)).unwrap();

    assert_eq!(decoded, reply);
    assert!(report.accepted());
}

#[test]
fn effective_capabilities_never_exceed_ceiling() {
    let cx = cx();
    let packet = request_packet(
        Expr::Symbol(Symbol::qualified("core", "Any")),
        vec![Symbol::qualified("ai", "run")],
    );
    let caps = effective_caps(&cx, &packet).unwrap();

    assert!(caps.contains(&CapabilityName::new("ai/run")));
    assert!(!caps.contains(&CapabilityName::new("bridge/given.materialize")));
}

#[test]
fn content_key_changes_when_request_changes() {
    let left = ContentKey::from_request(&eval_request("left"));
    let right = ContentKey::from_request(&eval_request("right"));

    assert_ne!(left, right);
}
