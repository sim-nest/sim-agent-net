use std::sync::Mutex;

use sim_citizen::CitizenField;
use sim_codec_bridge::{
    BridgeBook, BridgeCallPayload, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart,
    BridgeProvenance, encode_bridge_text, packet_content_id, packet_to_expr, stamp_packet_cid,
};
use sim_kernel::{
    ContentId, Cx, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol, testing::bare_cx as cx,
};
use sim_lib_agent_runner_core::{
    ModelResponse, OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA,
};
use sim_value::{access::field, build::entry};

use crate::{CompiledIntent, IntentStatus, LiftOptions, forge_lift_frontier, forge_lift_once};

fn content_id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), [byte; 32])
}

#[test]
fn default_compiled_intent_starts_as_candidate() {
    let intent = CompiledIntent::default();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert!(intent.compiler_card.is_none());
    assert!(intent.approval.is_none());
}

#[test]
fn verified_and_golden_statuses_are_distinct() {
    assert_ne!(IntentStatus::Verified, IntentStatus::Golden);
    assert_eq!(
        IntentStatus::from_symbol(&Symbol::qualified("forge", "verified")),
        Some(IntentStatus::Verified)
    );
    assert_eq!(
        IntentStatus::from_symbol(&Symbol::qualified("forge", "golden")),
        Some(IntentStatus::Golden)
    );
}

#[test]
fn status_round_trips_as_symbol_field() {
    let encoded = IntentStatus::Golden.encode_field();
    assert_eq!(encoded, Expr::Symbol(Symbol::qualified("forge", "golden")));

    let decoded = IntentStatus::decode_field_expr(&encoded, "status").unwrap();
    assert_eq!(decoded, IntentStatus::Golden);
}

#[test]
fn compiled_intent_keeps_content_ids_and_human_approval_separate() {
    let intent = CompiledIntent {
        name: Symbol::qualified("forge", "summarize"),
        version: 3,
        source: content_id(10),
        packet: content_id(11),
        verifiers: vec![Symbol::qualified("bridge", "vote")],
        probes: vec![content_id(12)],
        status: IntentStatus::Verified,
        compiler_card: Some(content_id(13)),
        approval: None,
    };

    assert_eq!(intent.source, content_id(10));
    assert_eq!(intent.packet, content_id(11));
    assert_eq!(intent.status, IntentStatus::Verified);
    assert_ne!(intent.status, IntentStatus::Golden);
    assert!(intent.approval.is_none());
}

struct ScriptedLiftFabric {
    responses: Mutex<Vec<Expr>>,
    requests: Mutex<Vec<Expr>>,
}

impl ScriptedLiftFabric {
    fn new(responses: Vec<Expr>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Expr> {
        self.requests.lock().unwrap().clone()
    }
}

impl EvalFabric for ScriptedLiftFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.requests.lock().unwrap().push(request.expr.clone());
        let parent_cid = bridge_cid_from_request(&request)?;
        let payload = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(sim_kernel::Error::Eval(
                    "scripted lift fabric is exhausted".to_owned(),
                ));
            }
            responses.remove(0)
        };
        let reply = stamp_packet_cid(&reply_packet(&parent_cid, payload))?;
        let response = ModelResponse::new(
            Symbol::qualified("runner", "forge-fixture"),
            "forge-fixture",
            vec![text_content(encode_bridge_text(
                &reply,
                &BridgeBook::standard(),
            )?)],
            Symbol::new("stop"),
        );
        Ok(EvalReply {
            value: cx.factory().expr(Expr::from(response))?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

struct ScriptedFrontierFabric {
    rows: Mutex<Vec<Expr>>,
    requests: Mutex<Vec<Expr>>,
}

impl ScriptedFrontierFabric {
    fn new(rows: Vec<Expr>) -> Self {
        Self {
            rows: Mutex::new(rows),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Expr> {
        self.requests.lock().unwrap().clone()
    }
}

impl EvalFabric for ScriptedFrontierFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.requests.lock().unwrap().push(request.expr.clone());
        let row = {
            let mut rows = self.rows.lock().unwrap();
            if rows.is_empty() {
                return Err(sim_kernel::Error::Eval(
                    "scripted frontier fabric is exhausted".to_owned(),
                ));
            }
            rows.remove(0)
        };
        let response = ModelResponse::new(
            Symbol::qualified("runner", "forge-frontier-fixture"),
            "forge-frontier-fixture",
            vec![Expr::Map(vec![
                entry(
                    "type",
                    Expr::Symbol(Symbol::qualified("forge", "FrontierPart")),
                ),
                entry("row", row),
            ])],
            Symbol::new("stop"),
        );
        Ok(EvalReply {
            value: cx.factory().expr(Expr::from(response))?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

fn bridge_cid_from_request(request: &EvalRequest) -> Result<String> {
    match field(&request.expr, "bridge-cid") {
        Some(Expr::String(cid)) => Ok(cid.clone()),
        _ => Err(sim_kernel::Error::Eval(
            "forge test request is missing bridge-cid".to_owned(),
        )),
    }
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        entry("type", Expr::Symbol(Symbol::new("text"))),
        entry("text", Expr::String(text)),
    ])
}

fn reply_packet(parent_cid: &str, payload: Expr) -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("reply"),
            from: "model:forge-lift".to_owned(),
            to: vec!["sim".to_owned()],
            role: Symbol::new("implementer"),
            parents: vec![parent_cid.to_owned()],
            task: Symbol::new("A1"),
            output: Symbol::new("A1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![BridgePart {
            id: Symbol::new("A1"),
            kind: Symbol::qualified("bridge", "Return"),
            payload,
        }],
        warrant: None,
    }
}

fn candidate_packet(return_shape: Expr) -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec!["model:worker".to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("C1"),
            output: Symbol::new("O1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("C1"),
                kind: Symbol::qualified("bridge", "Call"),
                payload: BridgeCallPayload::new(Symbol::qualified("forge", "answer")).to_expr(),
            },
            BridgePart {
                id: Symbol::new("O1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(Symbol::qualified("codec", "json"))),
                    entry("shape", return_shape),
                ]),
            },
        ],
        warrant: None,
    }
}

fn task_frame_part() -> BridgePart {
    BridgePart {
        id: Symbol::new("T1"),
        kind: Symbol::qualified("bridge", "Frame"),
        payload: BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
            .with_slot(Symbol::new("what"), Expr::Symbol(Symbol::new("summary")))
            .with_slot(
                Symbol::new("target"),
                Expr::Symbol(Symbol::new("transcript")),
            )
            .to_expr(),
    }
}

fn malformed_task_frame_part() -> BridgePart {
    BridgePart {
        id: Symbol::new("T1"),
        kind: Symbol::qualified("bridge", "Frame"),
        payload: BridgeFramePayload::new(Symbol::qualified("bridge", "produce-artifact"))
            .with_slot(Symbol::new("what"), Expr::Symbol(Symbol::new("summary")))
            .to_expr(),
    }
}

fn return_part(return_shape: Expr) -> BridgePart {
    BridgePart {
        id: Symbol::new("O1"),
        kind: Symbol::qualified("bridge", "Return"),
        payload: Expr::Map(vec![
            entry("codec", Expr::Symbol(Symbol::qualified("codec", "json"))),
            entry("shape", return_shape),
        ]),
    }
}

fn frontier_row(head: &str, part: BridgePart) -> Expr {
    Expr::Map(vec![
        entry("head", Expr::Symbol(Symbol::new(head))),
        entry(
            "part",
            Expr::Map(vec![
                entry("id", Expr::Symbol(part.id)),
                entry("kind", Expr::Symbol(part.kind)),
                entry("payload", part.payload),
            ]),
        ),
    ])
}

fn candidate_payload(return_shape: Expr) -> Expr {
    packet_to_expr(&candidate_packet(return_shape))
}

fn lift_options() -> LiftOptions {
    LiftOptions {
        name: Symbol::qualified("forge", "summarize"),
        max_repairs: 0,
    }
}

fn request_task_text(expr: &Expr) -> &str {
    match field(expr, "task") {
        Some(Expr::String(task)) => task,
        _ => panic!("request missing task text"),
    }
}

fn model_extra<'a>(request: &'a Expr, name: &str) -> Option<&'a Expr> {
    field(request, name)
}

#[test]
fn prose_compiles_to_packet_with_typed_return() {
    let mut cx = cx();
    let candidate = stamp_packet_cid(&candidate_packet(Expr::Symbol(Symbol::qualified(
        "core", "String",
    ))))
    .unwrap();
    let fabric = ScriptedLiftFabric::new(vec![packet_to_expr(&candidate)]);
    let intent = forge_lift_once(
        &mut cx,
        &fabric,
        "summarize the transcript",
        &lift_options(),
    )
    .unwrap();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert_eq!(intent.name, Symbol::qualified("forge", "summarize"));
    assert_eq!(intent.packet, packet_content_id(&candidate).unwrap());
}

#[test]
fn untypeable_prose_is_a_shape_obligation() {
    let mut cx = cx();
    let fabric = ScriptedLiftFabric::new(vec![candidate_payload(Expr::Bool(false))]);
    let err =
        forge_lift_once(&mut cx, &fabric, "do the untyped thing", &lift_options()).unwrap_err();

    assert!(err.to_string().contains("return Shape does not parse"));
}

#[test]
fn bounded_repair_fixes_near_miss() {
    let mut cx = cx();
    let fabric = ScriptedLiftFabric::new(vec![
        candidate_payload(Expr::Bool(false)),
        candidate_payload(Expr::Symbol(Symbol::qualified("core", "String"))),
    ]);
    let opts = LiftOptions {
        name: Symbol::qualified("forge", "repairable"),
        max_repairs: 1,
    };
    let intent = forge_lift_once(&mut cx, &fabric, "repair the packet", &opts).unwrap();
    let requests = fabric.requests();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert_eq!(requests.len(), 2);
    assert!(request_task_text(&requests[1]).contains("bridge-report"));
}

#[test]
fn unowned_normative_prose_fails_ownership() {
    let mut cx = cx();
    let fabric = ScriptedLiftFabric::new(vec![Expr::String("just do it".to_owned())]);
    let err = forge_lift_once(
        &mut cx,
        &fabric,
        "emit prose instead of a packet",
        &lift_options(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("bridge/Packet"));
}

#[test]
fn frontier_lift_authors_two_part_intent() {
    let mut cx = cx();
    let fabric = ScriptedFrontierFabric::new(vec![
        frontier_row("reply", task_frame_part()),
        frontier_row(
            "reply",
            return_part(Expr::Symbol(Symbol::qualified("core", "String"))),
        ),
    ]);
    let intent = forge_lift_frontier(
        &mut cx,
        &fabric,
        "summarize the transcript",
        &lift_options(),
    )
    .unwrap();
    let requests = fabric.requests();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert_eq!(intent.name, Symbol::qualified("forge", "summarize"));
    assert_eq!(requests.len(), 2);
}

#[test]
fn off_menu_part_rejected_with_valid_menu() {
    let mut cx = cx();
    let fabric = ScriptedFrontierFabric::new(vec![
        frontier_row("invented-head", task_frame_part()),
        frontier_row("reply", task_frame_part()),
        frontier_row(
            "reply",
            return_part(Expr::Symbol(Symbol::qualified("core", "String"))),
        ),
    ]);
    let intent = forge_lift_frontier(
        &mut cx,
        &fabric,
        "summarize the transcript",
        &lift_options(),
    )
    .unwrap();
    let requests = fabric.requests();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert_eq!(requests.len(), 3);
    assert!(format!("{:?}", model_extra(&requests[1], "forge-obligations")).contains("off-menu"));
    assert!(format!("{:?}", model_extra(&requests[1], "forge-obligations")).contains("reply"));
    assert!(format!("{:?}", model_extra(&requests[1], "forge-expected-part")).contains("T1"));
}

#[test]
fn off_shape_part_is_row_scoped_and_not_committed() {
    let mut cx = cx();
    let fabric = ScriptedFrontierFabric::new(vec![
        frontier_row("reply", malformed_task_frame_part()),
        frontier_row("reply", task_frame_part()),
        frontier_row(
            "reply",
            return_part(Expr::Symbol(Symbol::qualified("core", "String"))),
        ),
    ]);
    let intent = forge_lift_frontier(
        &mut cx,
        &fabric,
        "summarize the transcript",
        &lift_options(),
    )
    .unwrap();
    let requests = fabric.requests();

    assert_eq!(intent.status, IntentStatus::Candidate);
    assert_eq!(requests.len(), 3);
    assert!(
        format!("{:?}", model_extra(&requests[1], "forge-obligations"))
            .contains("frontier/rows/0/part")
    );
    assert!(format!("{:?}", model_extra(&requests[1], "forge-expected-part")).contains("T1"));
}

#[test]
fn typed_slot_never_offered_as_free_text() {
    let mut cx = cx();
    let fabric = ScriptedFrontierFabric::new(vec![
        frontier_row("reply", task_frame_part()),
        frontier_row(
            "reply",
            return_part(Expr::Symbol(Symbol::qualified("core", "String"))),
        ),
    ]);
    forge_lift_frontier(
        &mut cx,
        &fabric,
        "summarize the transcript",
        &lift_options(),
    )
    .unwrap();
    let requests = fabric.requests();
    let second = &requests[1];

    assert!(format!("{:?}", model_extra(second, "forge-frontier-slots")).contains("T1.what"));
    assert!(format!("{:?}", model_extra(second, "forge-frontier-slots")).contains("String"));
    assert_eq!(
        model_extra(second, OUTPUT_GRAMMAR_REQUIRED_EXTRA),
        Some(&Expr::Bool(true))
    );
    assert!(
        matches!(model_extra(second, OUTPUT_GRAMMAR_EXTRA), Some(Expr::String(grammar)) if grammar.contains("\"head\"") && grammar.contains("\"const\""))
    );
}
