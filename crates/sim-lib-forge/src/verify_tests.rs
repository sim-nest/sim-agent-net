use std::sync::Mutex;

use sim_codec_bridge::{
    BridgeBook, BridgeCallPayload, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart,
    BridgeProvenance, BridgeScore, BridgeVotePayload, encode_bridge_text, packet_to_expr,
    stamp_packet_cid,
};
use sim_kernel::{
    ContentId, Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, NumberLiteral, Result, Symbol,
    testing::bare_cx,
};
use sim_lib_agent_runner_core::ModelResponse;
use sim_value::{access::field, build::entry};

use crate::{
    CompiledIntent, ForgeResolver, IntentLibrary, IntentStatus, LiftOptions, ProbeOracle,
    PromotePolicy, Verifier, VerifyCatalog, VerifyProbe,
};

struct ScriptedLiftFabric {
    responses: Mutex<Vec<Expr>>,
}

impl ScriptedLiftFabric {
    fn new(responses: Vec<Expr>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

impl EvalFabric for ScriptedLiftFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        let parent_cid = bridge_cid_from_request(&request)?;
        let payload = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(Error::Eval(
                    "scripted verifier fabric is exhausted".to_owned(),
                ));
            }
            responses.remove(0)
        };
        let reply = stamp_packet_cid(&reply_packet(&parent_cid, payload))?;
        let response = ModelResponse::new(
            Symbol::qualified("runner", "forge-verify-fixture"),
            "forge-verify-fixture",
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

fn bridge_cid_from_request(request: &EvalRequest) -> Result<String> {
    match field(&request.expr, "bridge-cid") {
        Some(Expr::String(cid)) => Ok(cid.clone()),
        _ => Err(Error::Eval(
            "forge verify test request is missing bridge-cid".to_owned(),
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
            parents: vec![format!("{parent_cid}#move=request")],
            task: Symbol::new("A0"),
            output: Symbol::new("A1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("A0"),
                kind: Symbol::qualified("bridge", "Frame"),
                payload: BridgeFramePayload::new(Symbol::qualified("bridge", "answer")).to_expr(),
            },
            BridgePart {
                id: Symbol::new("A1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload,
            },
        ],
        warrant: None,
    }
}

fn candidate_packet(verifier_parts: Vec<BridgePart>) -> BridgePacket {
    let mut body = vec![
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
                entry("shape", Expr::Symbol(Symbol::qualified("core", "String"))),
            ]),
        },
    ];
    body.extend(verifier_parts);
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
        body,
        warrant: None,
    }
}

fn check_part(id: &str) -> BridgePart {
    BridgePart {
        id: Symbol::new(id),
        kind: Symbol::qualified("bridge", "Check"),
        payload: Expr::Map(vec![entry(
            "predicate",
            Expr::Symbol(Symbol::qualified("forge", "equals")),
        )]),
    }
}

fn vote_part(id: &str, target: &str) -> BridgePart {
    BridgePart {
        id: Symbol::new(id),
        kind: Symbol::qualified("bridge", "Vote"),
        payload: BridgeVotePayload::new(
            target,
            vec![BridgeScore::new(
                Symbol::new("correctness"),
                1,
                "answer matches the case",
            )],
        )
        .to_expr(),
    }
}

fn lift_options() -> LiftOptions {
    LiftOptions {
        name: Symbol::qualified("forge", "summarize"),
        max_repairs: 0,
    }
}

fn content_id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), [byte; 32])
}

fn intent_with_verifiers(verifiers: Vec<Symbol>) -> CompiledIntent {
    CompiledIntent {
        name: Symbol::qualified("forge", "summarize"),
        version: 1,
        source: content_id(1),
        packet: content_id(2),
        verifiers,
        probes: Vec::new(),
        status: IntentStatus::Candidate,
        compiler_card: None,
        approval: None,
    }
}

fn number(value: i64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("core", "Integer"),
        canonical: value.to_string(),
    })
}

fn field_between_predicate(field_name: &str, min: i64, max: i64) -> Expr {
    Expr::Map(vec![
        entry(
            "predicate",
            Expr::Symbol(Symbol::qualified("forge", "field-number-between")),
        ),
        entry("field", Expr::String(field_name.to_owned())),
        entry("min", number(min)),
        entry("max", number(max)),
    ])
}

fn equals_predicate(expected: Expr) -> Expr {
    Expr::Map(vec![
        entry(
            "predicate",
            Expr::Symbol(Symbol::qualified("forge", "equals")),
        ),
        entry("expected", expected),
    ])
}

fn verifier_probe(id: Symbol, expected: Expr) -> VerifyProbe {
    VerifyProbe {
        args: Expr::List(Vec::new()),
        oracle: ProbeOracle::Expected(expected),
        verifier_ids: vec![id],
    }
}

fn resolver_with(catalog: VerifyCatalog) -> ForgeResolver {
    ForgeResolver::new_with_verifiers(IntentLibrary::new(), lift_options(), catalog)
}

#[test]
fn assertion_rejects_out_of_bounds_well_formed_answer() {
    let mut cx = bare_cx();
    let verifier_id = Symbol::new("A1");
    let mut catalog = VerifyCatalog::new();
    catalog.register_verifier(
        verifier_id.clone(),
        Verifier::Assertion {
            predicate: field_between_predicate("score", 0, 10),
        },
    );
    let intent = intent_with_verifiers(vec![verifier_id.clone()]);
    let answer = Expr::Map(vec![entry("score", number(42))]);

    let report = catalog.verify_answer(&mut cx, &intent, &answer).unwrap();

    assert!(!report.accepted());
    assert_eq!(report.failed[0].id, verifier_id);
    assert!(report.failed[0].reason.contains("above maximum"));
}

#[test]
fn passing_probe_promotes_candidate_to_verified() {
    let mut cx = bare_cx();
    let verifier_id = Symbol::new("A1");
    let candidate = stamp_packet_cid(&candidate_packet(vec![check_part("A1")])).unwrap();
    let fabric = ScriptedLiftFabric::new(vec![packet_to_expr(&candidate)]);
    let mut catalog = VerifyCatalog::new();
    catalog.register_verifier(
        verifier_id.clone(),
        Verifier::Assertion {
            predicate: equals_predicate(Expr::String("ok".to_owned())),
        },
    );
    let probe_id = catalog
        .register_probe(
            lift_options().name,
            verifier_probe(verifier_id, Expr::String("ok".to_owned())),
        )
        .unwrap();
    let mut resolver = resolver_with(catalog);

    let resolved = resolver
        .resolve(
            &mut cx,
            &fabric,
            "summarize the transcript",
            PromotePolicy::AutoVerifiedOnProbePass,
        )
        .unwrap();

    assert_eq!(resolved.status, IntentStatus::Verified);
    assert_eq!(resolved.probes, vec![probe_id]);
    assert!(resolved.approval.is_none());
}

#[test]
fn judge_below_quorum_blocks_promotion() {
    let mut cx = bare_cx();
    let verifier_id = Symbol::new("J1");
    let candidate = stamp_packet_cid(&candidate_packet(vec![vote_part("J1", "answer")])).unwrap();
    let fabric = ScriptedLiftFabric::new(vec![packet_to_expr(&candidate)]);
    let mut catalog = VerifyCatalog::new();
    catalog.register_verifier(
        verifier_id.clone(),
        Verifier::Judge {
            seat: "judge:one".to_owned(),
            packet: Box::new(judge_vote_packet("judge:one", "answer", 1)),
            reply_to: Some(Box::new(judge_parent_packet())),
            target: "answer".to_owned(),
            min_votes: 2,
        },
    );
    catalog
        .register_probe(
            lift_options().name,
            verifier_probe(verifier_id, Expr::String("ok".to_owned())),
        )
        .unwrap();
    let mut resolver = resolver_with(catalog);

    let resolved = resolver
        .resolve(
            &mut cx,
            &fabric,
            "summarize the transcript",
            PromotePolicy::AutoVerifiedOnProbePass,
        )
        .unwrap();

    assert_eq!(resolved.status, IntentStatus::Candidate);
    assert!(!resolved.probes.is_empty());
}

#[test]
fn evidence_absent_fails_closed() {
    let mut cx = bare_cx();
    let verifier_id = Symbol::new("E1");
    let mut catalog = VerifyCatalog::new();
    catalog.register_verifier(
        verifier_id.clone(),
        Verifier::Evidence {
            cites: vec!["given:gold".to_owned()],
        },
    );
    let intent = intent_with_verifiers(vec![verifier_id]);

    let report = catalog
        .verify_answer(&mut cx, &intent, &Expr::String("ok".to_owned()))
        .unwrap();

    assert!(!report.accepted());
    assert!(report.failed[0].reason.contains("absent"));
}

#[test]
fn missing_probe_blocks_auto_verified_promotion() {
    let mut cx = bare_cx();
    let verifier_id = Symbol::new("A1");
    let candidate = stamp_packet_cid(&candidate_packet(vec![check_part("A1")])).unwrap();
    let fabric = ScriptedLiftFabric::new(vec![packet_to_expr(&candidate)]);
    let mut catalog = VerifyCatalog::new();
    catalog.register_verifier(
        verifier_id,
        Verifier::Assertion {
            predicate: equals_predicate(Expr::String("ok".to_owned())),
        },
    );
    let mut resolver = resolver_with(catalog);

    let resolved = resolver
        .resolve(
            &mut cx,
            &fabric,
            "summarize the transcript",
            PromotePolicy::AutoVerifiedOnProbePass,
        )
        .unwrap();

    assert_eq!(resolved.status, IntentStatus::Candidate);
    assert!(resolved.probes.is_empty());
}

fn judge_parent_packet() -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: Some("core/sha256-bridge-v1:parent".to_owned()),
            move_kind: Symbol::new("reply"),
            from: "model:worker".to_owned(),
            to: vec!["sim".to_owned()],
            role: Symbol::new("implementer"),
            parents: vec!["core/sha256-bridge-v1:root#move=request".to_owned()],
            task: Symbol::new("O0"),
            output: Symbol::new("O1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("O0"),
                kind: Symbol::qualified("bridge", "Frame"),
                payload: BridgeFramePayload::new(Symbol::qualified("bridge", "answer")).to_expr(),
            },
            BridgePart {
                id: Symbol::new("O1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(Symbol::qualified("codec", "json"))),
                    entry("shape", Expr::Symbol(Symbol::qualified("core", "Any"))),
                ]),
            },
        ],
        warrant: None,
    }
}

fn judge_vote_packet(seat: &str, target: &str, votes: u32) -> BridgePacket {
    let mut body = Vec::new();
    for index in 0..votes {
        body.push(vote_part(&format!("V{}", index + 1), target));
    }
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("vote"),
            from: seat.to_owned(),
            to: vec!["sim".to_owned()],
            role: Symbol::new("reviewer"),
            parents: vec!["core/sha256-bridge-v1:parent#move=reply".to_owned()],
            task: Symbol::new("V1"),
            output: Symbol::new("V1"),
            ceiling: vec![Symbol::qualified("ai", "run")],
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body,
        warrant: None,
    }
}
