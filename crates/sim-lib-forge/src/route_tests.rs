use std::sync::Mutex;

use sim_codec_bridge::{
    BridgeCallPayload, BridgeHeader, BridgePacket, BridgePart, BridgeProvenance, packet_content_id,
    stamp_packet_cid,
};
use sim_kernel::{
    ContentId, Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, NumberLiteral, Result, Symbol,
    testing::bare_cx,
};
use sim_lib_agent_runner_core::ModelResponse;
use sim_value::build::entry;

use crate::{
    CompiledIntent, IntentStatus, RouteAttemptStatus, RoutePolicy, RouteTarget, Verifier,
    VerifyCatalog, run_intent_routed_report, store_packet_artifact,
};

struct ScriptedAnswerFabric {
    responses: Mutex<Vec<Expr>>,
    requests: Mutex<Vec<Expr>>,
}

impl ScriptedAnswerFabric {
    fn new(responses: Vec<Expr>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl EvalFabric for ScriptedAnswerFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.requests.lock().unwrap().push(request.expr);
        let response = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(Error::Eval(
                    "scripted answer fabric is exhausted".to_owned(),
                ));
            }
            responses.remove(0)
        };
        let response = ModelResponse::new(
            Symbol::qualified("runner", "forge-route-fixture"),
            "forge-route-fixture",
            vec![text_content(
                sim_codec_json::expr_to_json(&response).to_string(),
            )],
            Symbol::new("stop"),
        );
        Ok(EvalReply {
            value: cx.factory().expr(Expr::from(response))?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

fn text_content(text: String) -> Expr {
    Expr::Map(vec![
        entry("type", Expr::Symbol(Symbol::new("text"))),
        entry("text", Expr::String(text)),
    ])
}

fn route_cx() -> Cx {
    let mut cx = bare_cx();
    let json = sim_codec_json::JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    cx
}

fn request_packet(ceiling: Vec<Symbol>) -> BridgePacket {
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
            ceiling,
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
                    entry("shape", Expr::Symbol(Symbol::qualified("core", "String"))),
                ]),
            },
        ],
        warrant: None,
    }
}

fn intent_with_verifiers(
    cx: &mut Cx,
    verifiers: Vec<Symbol>,
    ceiling: Vec<Symbol>,
) -> (CompiledIntent, BridgePacket) {
    let packet = stamp_packet_cid(&request_packet(ceiling)).unwrap();
    let packet_id = store_packet_artifact(cx, &packet).unwrap();
    assert_eq!(packet_id, packet_content_id(&packet).unwrap());
    (
        CompiledIntent {
            name: Symbol::qualified("forge", "route"),
            version: 1,
            source: content_id(1),
            packet: packet_id,
            verifiers,
            probes: Vec::new(),
            status: IntentStatus::Verified,
            compiler_card: None,
            approval: None,
        },
        packet,
    )
}

fn verifier_catalog(expected: &str) -> VerifyCatalog {
    let mut catalog = VerifyCatalog::new();
    catalog.register_verifier(
        Symbol::new("A1"),
        Verifier::Assertion {
            predicate: Expr::Map(vec![
                entry(
                    "predicate",
                    Expr::Symbol(Symbol::qualified("forge", "equals")),
                ),
                entry("expected", Expr::String(expected.to_owned())),
            ]),
        },
    );
    catalog
}

fn content_id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), [byte; 32])
}

#[test]
fn cheap_failure_escalates_to_strong() {
    let mut cx = route_cx();
    let (intent, _) = intent_with_verifiers(&mut cx, vec![Symbol::new("A1")], Vec::new());
    let cheap = ScriptedAnswerFabric::new(vec![Expr::String("wrong".to_owned())]);
    let strong = ScriptedAnswerFabric::new(vec![Expr::String("strong".to_owned())]);
    let policy = RoutePolicy::new(
        vec![
            RouteTarget::new("cheap", &cheap),
            RouteTarget::new("strong", &strong).with_card("card:strong"),
        ],
        1,
    )
    .with_repair_retries(0)
    .with_verify_catalog(verifier_catalog("strong"));

    let report =
        run_intent_routed_report(&mut cx, &intent, &Expr::String("args".to_owned()), &policy)
            .unwrap();

    assert_eq!(report.answer, Expr::String("strong".to_owned()));
    assert_eq!(report.provenance.target, "strong");
    assert_eq!(report.provenance.card, Some("card:strong".to_owned()));
    assert_eq!(cheap.request_count(), 1);
    assert_eq!(strong.request_count(), 1);
    assert!(matches!(
        report.attempts[0].status,
        RouteAttemptStatus::Failed
    ));
    assert!(matches!(
        report.attempts[1].status,
        RouteAttemptStatus::Accepted
    ));
}

#[test]
fn cheap_success_never_escalates() {
    let mut cx = route_cx();
    let (intent, _) = intent_with_verifiers(&mut cx, vec![Symbol::new("A1")], Vec::new());
    let cheap = ScriptedAnswerFabric::new(vec![Expr::String("ok".to_owned())]);
    let strong = ScriptedAnswerFabric::new(vec![Expr::String("unused".to_owned())]);
    let policy = RoutePolicy::new(
        vec![
            RouteTarget::new("cheap", &cheap),
            RouteTarget::new("strong", &strong),
        ],
        1,
    )
    .with_repair_retries(0)
    .with_verify_catalog(verifier_catalog("ok"));

    let report = run_intent_routed_report(&mut cx, &intent, &Expr::Nil, &policy).unwrap();

    assert_eq!(report.answer, Expr::String("ok".to_owned()));
    assert_eq!(report.provenance.target, "cheap");
    assert_eq!(cheap.request_count(), 1);
    assert_eq!(strong.request_count(), 0);
}

#[test]
fn target_repair_precedes_escalation() {
    let mut cx = route_cx();
    let (intent, _) = intent_with_verifiers(&mut cx, Vec::new(), Vec::new());
    let cheap = ScriptedAnswerFabric::new(vec![
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("number", "i64"),
            canonical: "9".to_owned(),
        }),
        Expr::String("repaired".to_owned()),
    ]);
    let strong = ScriptedAnswerFabric::new(vec![Expr::String("unused".to_owned())]);
    let policy = RoutePolicy::new(
        vec![
            RouteTarget::new("cheap", &cheap),
            RouteTarget::new("strong", &strong),
        ],
        1,
    )
    .with_repair_retries(1);

    let report = run_intent_routed_report(&mut cx, &intent, &Expr::Nil, &policy).unwrap();

    assert_eq!(report.answer, Expr::String("repaired".to_owned()));
    assert_eq!(report.provenance.target, "cheap");
    assert_eq!(cheap.request_count(), 2);
    assert_eq!(strong.request_count(), 0);
}

#[test]
fn routed_call_respects_ceiling() {
    let mut cx = route_cx();
    let (intent, _) = intent_with_verifiers(&mut cx, Vec::new(), Vec::new());
    let skipped = ScriptedAnswerFabric::new(vec![Expr::String("skip".to_owned())]);
    let allowed = ScriptedAnswerFabric::new(vec![Expr::String("ok".to_owned())]);
    let policy = RoutePolicy::new(
        vec![
            RouteTarget::new("over-ceiling", &skipped)
                .requiring(vec![Symbol::qualified("fs", "read")]),
            RouteTarget::new("allowed", &allowed),
        ],
        1,
    )
    .with_repair_retries(0);

    let report = run_intent_routed_report(&mut cx, &intent, &Expr::Nil, &policy).unwrap();

    assert_eq!(report.answer, Expr::String("ok".to_owned()));
    assert_eq!(skipped.request_count(), 0);
    assert_eq!(allowed.request_count(), 1);
    assert!(matches!(
        report.attempts[0].status,
        RouteAttemptStatus::Skipped
    ));
    assert_eq!(report.provenance.target, "allowed");
}
