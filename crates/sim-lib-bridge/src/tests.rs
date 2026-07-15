use std::sync::{Arc, Mutex};

use sim_codec_bridge::{
    BridgeBook, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart, BridgePatchPayload,
    BridgeProvenance, BridgeScore, BridgeVotePayload, assert_total_ownership, encode_bridge_text,
    stamp_packet_cid,
};
use sim_codec_json::JsonCodecLib;
use sim_kernel::{
    Args, Callable, CapabilityName, Consistency, Cx, DefaultFactory, EagerPolicy, Error,
    EvalFabric, EvalMode, EvalReply, EvalRequest, Export, Expr, Lib, Result, Symbol,
};
use sim_lib_agent_runner_core::ModelResponse;
use sim_lib_stream_fabric::{ContentKey, EvalCassette, EvalCassetteLedger, LedgeredRelayFabric};
use sim_value::build::entry;

mod ask;
mod loom;

use crate::{
    BridgeFunction, BridgeFunctionKind, BridgeLib, MergePolicy, bridge_brief, bridge_brief_symbol,
    bridge_request_content_key, bridge_rx_response, bridge_tx, effective_caps, frontier,
    merge_bridge_replies, render_model_face, run_bridge, rx_check,
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
    sim_test_support::register_core_classes(&mut cx);
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
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

fn collaboration_base_packet() -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("reply"),
            from: "model:drafter".to_owned(),
            to: vec!["human:reviewer".to_owned(), "model:judge".to_owned()],
            role: Symbol::new("implementer"),
            parents: vec!["core/sha256-bridge-v1:root".to_owned()],
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
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(Symbol::qualified("codec", "bridge"))),
                    entry("shape", Expr::Symbol(Symbol::qualified("core", "Map"))),
                ]),
            },
        ],
        warrant: None,
    }
}

fn patch_reply(parent: &BridgePacket, from: &str, replacement: Expr) -> BridgePacket {
    let parent_cid = parent.header.cid.clone().unwrap();
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("patch"),
            from: from.to_owned(),
            to: vec!["sim".to_owned()],
            role: Symbol::new("reviewer"),
            parents: vec![parent_cid.clone()],
            task: Symbol::new("P1"),
            output: Symbol::new("P1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![BridgePart {
            id: Symbol::new("P1"),
            kind: Symbol::qualified("bridge", "Patch"),
            payload: BridgePatchPayload::new(parent_cid, "body/O2/payload", replacement).to_expr(),
        }],
        warrant: None,
    }
}

fn vote_reply(parent: &BridgePacket, from: &str) -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("vote"),
            from: from.to_owned(),
            to: vec!["sim".to_owned()],
            role: Symbol::new("judge"),
            parents: vec![parent.header.cid.clone().unwrap()],
            task: Symbol::new("V1"),
            output: Symbol::new("V1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![BridgePart {
            id: Symbol::new("V1"),
            kind: Symbol::qualified("bridge", "Vote"),
            payload: BridgeVotePayload::new(
                "body/O2/payload",
                vec![BridgeScore::new(
                    Symbol::new("correctness"),
                    1,
                    "keeps the packet contract",
                )],
            )
            .to_expr(),
        }],
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
fn collaboration_is_packet_based() {
    let base = stamp_packet_cid(&collaboration_base_packet()).unwrap();
    let patch = stamp_packet_cid(&patch_reply(
        &base,
        "human:reviewer",
        Expr::String("accepted answer".to_owned()),
    ))
    .unwrap();
    let merged = merge_bridge_replies(&base, &[patch], &MergePolicy::Single).unwrap();
    let patch = BridgePatchPayload::from_expr(&merged.body[0].payload).unwrap();

    assert_eq!(merged.header.move_kind, Symbol::new("patch"));
    assert_eq!(patch.parent_cid, base.header.cid.unwrap());
    assert_eq!(patch.target, "body/O2/payload");
    assert_eq!(
        patch.replacement,
        Expr::String("accepted answer".to_owned())
    );
}

#[test]
fn merge_still_satisfies_root_return() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let base = stamp_packet_cid(&collaboration_base_packet()).unwrap();
    let patch = stamp_packet_cid(&patch_reply(
        &base,
        "model:synthesizer",
        Expr::String("accepted answer".to_owned()),
    ))
    .unwrap();
    let vote = stamp_packet_cid(&vote_reply(&base, "model:judge")).unwrap();
    let merged = merge_bridge_replies(
        &base,
        &[patch, vote],
        &MergePolicy::SynthesisThenVote {
            synthesizer: "model:synthesizer".to_owned(),
            min_votes: 1,
        },
    )
    .unwrap();
    let report = rx_check(&mut cx, &book, &merged, Some(&base)).unwrap();

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

#[test]
fn one_frame_record_yields_both_faces() {
    let book = BridgeBook::standard();
    let packet = stamp_packet_cid(
        &bridge_brief(
            "model:drafter",
            BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
                .with_slot(
                    Symbol::new("what"),
                    Expr::Symbol(Symbol::qualified("bridge", "proposal")),
                )
                .with_slot(
                    Symbol::new("target"),
                    Expr::String("sim-human-model".to_owned()),
                ),
            Expr::Symbol(Symbol::qualified("core", "String")),
        )
        .unwrap(),
    )
    .unwrap();
    let canonical = encode_bridge_text(&packet, &book).unwrap();
    let (face, spans) = render_model_face(&book, &packet).unwrap();

    assert!(canonical.contains("FRAME T1 payload="));
    assert!(face.starts_with(&canonical));
    assert!(face.contains("FLUENT"));
    assert!(face.contains("[T1] You MUST produce bridge/proposal for sim-human-model."));
    assert_total_ownership(&face, &spans).unwrap();
}

#[test]
fn frontier_emits_flat_oneof_that_lowers() {
    let mut cx = cx();
    let packet = bridge_brief(
        "model:drafter",
        BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
            .with_slot(
                Symbol::new("what"),
                Expr::Symbol(Symbol::qualified("bridge", "proposal")),
            )
            .with_slot(
                Symbol::new("target"),
                Expr::String("sim-human-model".to_owned()),
            ),
        Expr::Symbol(Symbol::qualified("core", "String")),
    )
    .unwrap();
    let menu = frontier(&mut cx, &packet).unwrap();

    assert_eq!(menu.slots.len(), 2);
    assert!(format!("{:?}", menu.heads).contains("reply"));
    assert!(menu.grammar.contains(r#""anyOf""#));
    assert!(menu.grammar.contains("reply"));
}

#[test]
fn bridge_brief_runtime_export_constructs_packet() {
    let mut cx = cx();
    let exported = BridgeLib
        .manifest()
        .exports
        .iter()
        .any(|export| matches!(export, Export::Function { symbol, .. } if *symbol == bridge_brief_symbol()));
    assert!(exported);

    let target = cx
        .factory()
        .expr(Expr::String("model:drafter".to_owned()))
        .unwrap();
    let frame = cx
        .factory()
        .expr(
            BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
                .with_slot(
                    Symbol::new("what"),
                    Expr::Symbol(Symbol::qualified("bridge", "proposal")),
                )
                .with_slot(
                    Symbol::new("target"),
                    Expr::String("sim-human-model".to_owned()),
                )
                .to_expr(),
        )
        .unwrap();
    let return_shape = cx
        .factory()
        .expr(Expr::Symbol(Symbol::qualified("core", "String")))
        .unwrap();

    let value = BridgeFunction::new(BridgeFunctionKind::Brief)
        .call(&mut cx, Args::new(vec![target, frame, return_shape]))
        .unwrap();
    let packet =
        sim_codec_bridge::expr_to_packet(&value.object().as_expr(&mut cx).unwrap()).unwrap();

    assert_eq!(packet.header.to, vec!["model:drafter".to_owned()]);
    assert_eq!(packet.body[0].kind, Symbol::qualified("bridge", "Frame"));
    assert_eq!(packet.body[1].kind, Symbol::qualified("bridge", "Return"));
}
