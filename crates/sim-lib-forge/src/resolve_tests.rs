use std::sync::Mutex;

use sim_codec_bridge::{
    BridgeBook, BridgeCallPayload, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart,
    BridgeProvenance, encode_bridge_text, packet_content_id, packet_to_expr, stamp_packet_cid,
};
use sim_kernel::{
    ContentId, Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol,
    testing::bare_cx,
};
use sim_lib_agent_runner_core::ModelResponse;
use sim_value::{access::field, build::entry};

use crate::{
    CompiledIntent, IntentLibrary, IntentStatus, LiftOptions, PromotePolicy,
    forge_resolve_with_options, normalize_prose,
};

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
                return Err(Error::Eval(
                    "scripted resolve fabric is exhausted".to_owned(),
                ));
            }
            responses.remove(0)
        };
        let reply = stamp_packet_cid(&reply_packet(&parent_cid, payload))?;
        let response = ModelResponse::new(
            Symbol::qualified("runner", "forge-resolve-fixture"),
            "forge-resolve-fixture",
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

#[derive(Default)]
struct FailingLiftFabric {
    requests: Mutex<Vec<Expr>>,
}

impl FailingLiftFabric {
    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl EvalFabric for FailingLiftFabric {
    fn realize(&self, _cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.requests.lock().unwrap().push(request.expr);
        Err(Error::Eval(
            "golden resolve hit must not call lift fabric".to_owned(),
        ))
    }
}

fn bridge_cid_from_request(request: &EvalRequest) -> Result<String> {
    match field(&request.expr, "bridge-cid") {
        Some(Expr::String(cid)) => Ok(cid.clone()),
        _ => Err(Error::Eval(
            "forge resolve test request is missing bridge-cid".to_owned(),
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

fn candidate_payload(return_shape: Expr) -> Expr {
    packet_to_expr(&candidate_packet(return_shape))
}

fn content_id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), [byte; 32])
}

fn lift_options() -> LiftOptions {
    LiftOptions {
        name: Symbol::qualified("forge", "summarize"),
        max_repairs: 0,
    }
}

fn golden_intent(source: ContentId, packet: ContentId, version: u32) -> CompiledIntent {
    CompiledIntent {
        name: Symbol::qualified("forge", "summarize"),
        version,
        source,
        packet,
        verifiers: Vec::new(),
        probes: Vec::new(),
        status: IntentStatus::Golden,
        compiler_card: None,
        approval: Some(content_id(200)),
    }
}

#[test]
fn normalize_prose_collapses_ascii_whitespace_and_preserves_case() {
    let (normalized, source) = normalize_prose("  Summarize\tFoo\nBar  ").unwrap();
    let (same, same_source) = normalize_prose("Summarize Foo Bar").unwrap();
    let (different_case, different_source) = normalize_prose("summarize Foo Bar").unwrap();

    assert_eq!(normalized, "Summarize Foo Bar");
    assert_eq!(same, normalized);
    assert_eq!(same_source, source);
    assert_eq!(different_case, "summarize Foo Bar");
    assert_ne!(different_source, source);
}

#[test]
fn golden_artifact_hit_skips_lift_runner() {
    let mut cx = bare_cx();
    let (_, source) = normalize_prose("summarize the transcript").unwrap();
    let golden = golden_intent(source, content_id(40), 3);
    let mut library = IntentLibrary::new();
    library.store(golden.clone()).unwrap();
    let fabric = FailingLiftFabric::default();

    let resolved = forge_resolve_with_options(
        &mut library,
        &lift_options(),
        &mut cx,
        &fabric,
        " summarize \n the\ttranscript ",
        PromotePolicy::KeepCandidate,
    )
    .unwrap();

    assert_eq!(resolved, golden);
    assert_eq!(fabric.request_count(), 0);
}

#[test]
fn relift_does_not_clobber_golden() {
    let mut cx = bare_cx();
    let (_, source) = normalize_prose("summarize the transcript").unwrap();
    let golden = golden_intent(source, content_id(41), 1);
    let mut library = IntentLibrary::new();
    library.store(golden.clone()).unwrap();
    let fabric = ScriptedLiftFabric::new(vec![candidate_payload(Expr::Symbol(Symbol::qualified(
        "core", "String",
    )))]);

    let resolved = forge_resolve_with_options(
        &mut library,
        &lift_options(),
        &mut cx,
        &fabric,
        "summarize the transcript",
        PromotePolicy::KeepCandidate,
    )
    .unwrap();

    assert_eq!(resolved, golden);
    assert_eq!(fabric.requests().len(), 0);
    assert_eq!(
        library.fetch(&Symbol::qualified("forge", "summarize"), 1),
        Some(&golden)
    );
}

#[test]
fn changed_prose_normal_form_recompiles() {
    let mut cx = bare_cx();
    let (_, old_source) = normalize_prose("summarize the transcript").unwrap();
    let (_, new_source) = normalize_prose("summarize the transcript now").unwrap();
    let golden = golden_intent(old_source.clone(), content_id(42), 1);
    let candidate = stamp_packet_cid(&candidate_packet(Expr::Symbol(Symbol::qualified(
        "core", "String",
    ))))
    .unwrap();
    let mut library = IntentLibrary::new();
    library.store(golden.clone()).unwrap();
    let fabric = ScriptedLiftFabric::new(vec![packet_to_expr(&candidate)]);

    let resolved = forge_resolve_with_options(
        &mut library,
        &lift_options(),
        &mut cx,
        &fabric,
        "summarize the transcript now",
        PromotePolicy::KeepCandidate,
    )
    .unwrap();

    assert_eq!(fabric.requests().len(), 1);
    assert_eq!(resolved.status, IntentStatus::Candidate);
    assert_eq!(resolved.source, new_source);
    assert_eq!(resolved.packet, packet_content_id(&candidate).unwrap());
    assert_eq!(resolved.version, 2);
    assert_eq!(library.by_source(&old_source), vec![&golden]);
    assert_eq!(library.by_source(&new_source), vec![&resolved]);
}

#[test]
fn rx_check_only_leaves_candidate() {
    let mut cx = bare_cx();
    let (_, source) = normalize_prose("summarize the transcript").unwrap();
    let candidate = stamp_packet_cid(&candidate_packet(Expr::Symbol(Symbol::qualified(
        "core", "String",
    ))))
    .unwrap();
    let mut library = IntentLibrary::new();
    let fabric = ScriptedLiftFabric::new(vec![packet_to_expr(&candidate)]);

    let resolved = forge_resolve_with_options(
        &mut library,
        &lift_options(),
        &mut cx,
        &fabric,
        "summarize the transcript",
        PromotePolicy::AutoVerifiedOnProbePass,
    )
    .unwrap();

    assert_eq!(resolved.status, IntentStatus::Candidate);
    assert_eq!(resolved.approval, None);
    assert_eq!(library.by_source(&source), vec![&resolved]);
}
