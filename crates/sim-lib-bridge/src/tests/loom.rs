use std::sync::Mutex;

use sim_codec_bridge::{
    BridgeBook, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart, BridgeProvenance,
    BridgeWeavePayload, BridgeWeaveRow, stamp_packet_cid,
};
use sim_kernel::{Cx, Error, EvalFabric, EvalReply, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{
    ModelRequest, ModelResponse, OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA,
    OUTPUT_GRAMMAR_REQUIRED_EXTRA,
};
use sim_value::build::entry;

use crate::{next_frontier_menu, rx_check, validate_weave, validate_woven_row, weave_row_by_row};

use super::{cx, text_content};

struct RowFabric {
    rows: Mutex<Vec<BridgeWeaveRow>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl RowFabric {
    fn new(rows: Vec<BridgeWeaveRow>) -> Self {
        Self {
            rows: Mutex::new(rows),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl EvalFabric for RowFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        self.requests
            .lock()
            .unwrap()
            .push(ModelRequest::try_from(request.expr.clone())?);
        let row = {
            let mut rows = self.rows.lock().unwrap();
            if rows.is_empty() {
                return Err(Error::Eval("row fabric is exhausted".to_owned()));
            }
            rows.remove(0)
        };
        let response = ModelResponse::new(
            Symbol::qualified("runner", "fixture"),
            "fixture",
            vec![
                text_content("draft".to_owned()),
                Expr::Map(vec![
                    entry(
                        "type",
                        Expr::Symbol(Symbol::qualified("bridge", "WeaveRow")),
                    ),
                    entry("row", row.to_expr()),
                ]),
            ],
            Symbol::new("stop"),
        );
        Ok(EvalReply {
            value: cx.factory().expr(Expr::from(response))?,
            diagnostics: Vec::new(),
            trace: None,
        })
    }
}

fn row(slot: &str, head: &str, roles: Vec<(&str, Expr)>) -> BridgeWeaveRow {
    BridgeWeaveRow::new(
        slot,
        Symbol::new(head),
        roles
            .into_iter()
            .map(|(role, value)| (Symbol::new(role), value))
            .collect(),
    )
}

fn weave_packet(rows: Vec<BridgeWeaveRow>) -> BridgePacket {
    BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec!["model:drafter".to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("W1"),
            output: Symbol::new("O1"),
            ceiling: vec![Symbol::qualified("ai", "run")],
            context: vec![Symbol::new("T1")],
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            BridgePart {
                id: Symbol::new("T1"),
                kind: Symbol::qualified("bridge", "Frame"),
                payload: BridgeFramePayload::new(Symbol::qualified("bridge", "proposal")).to_expr(),
            },
            BridgePart {
                id: Symbol::new("W1"),
                kind: Symbol::qualified("bridge", "Weave"),
                payload: BridgeWeavePayload::new(rows).to_expr(),
            },
            BridgePart {
                id: Symbol::new("O1"),
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

fn first_weave(packet: &BridgePacket) -> BridgeWeavePayload {
    BridgeWeavePayload::from_expr(&packet.body[1].payload).unwrap()
}

fn extra<'a>(request: &'a ModelRequest, name: &str) -> Option<&'a Expr> {
    request.extra.iter().find_map(|(key, value)| {
        matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == name).then_some(value)
    })
}

#[test]
fn valid_weave_fixture_passes() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let packet = stamp_packet_cid(&weave_packet(vec![row(
        "answer",
        "reply",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]))
    .unwrap();
    let report = rx_check(&mut cx, &book, &packet, None).unwrap();
    let loom_report = validate_weave(&mut cx, &packet, &first_weave(&packet)).unwrap();
    let menu = next_frontier_menu(&mut cx, &packet).unwrap();

    assert!(report.accepted());
    assert!(loom_report.accepted());
    assert!(format!("{:?}", menu.heads).contains("reply"));
}

#[test]
fn off_menu_head_is_row_scoped_obligation() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let packet = stamp_packet_cid(&weave_packet(vec![row(
        "answer",
        "invented-head",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]))
    .unwrap();
    let report = rx_check(&mut cx, &book, &packet, None).unwrap();

    assert!(!report.accepted());
    assert!(report.obligations.iter().any(|obligation| {
        obligation.path == "loom/rows/0/head" && obligation.reason.contains("off-menu")
    }));
}

#[test]
fn forward_ref_is_row_scoped_obligation() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let packet = stamp_packet_cid(&weave_packet(vec![
        row(
            "first",
            "reply",
            vec![("input", Expr::String("@second".to_owned()))],
        ),
        row(
            "second",
            "reply",
            vec![("input", Expr::String("@first".to_owned()))],
        ),
    ]))
    .unwrap();
    let report = rx_check(&mut cx, &book, &packet, None).unwrap();

    assert!(!report.accepted());
    assert!(report.obligations.iter().any(|obligation| {
        obligation.path == "loom/rows/0/roles/input"
            && obligation.reason.contains("forward reference")
    }));
}

#[test]
fn hand_written_result_shape_that_disagrees_rejects() {
    let mut cx = cx();
    let book = BridgeBook::standard();
    let mut packet = weave_packet(vec![row(
        "answer",
        "reply",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]);
    let Expr::Map(fields) = &mut packet.body[1].payload else {
        panic!("weave payload must be a map");
    };
    for (key, value) in fields {
        if matches!(key, Expr::Symbol(symbol) if symbol.name.as_ref() == "result-shape") {
            *value = Expr::Symbol(Symbol::qualified("core", "String"));
        }
    }
    let packet = stamp_packet_cid(&packet).unwrap();
    let report = rx_check(&mut cx, &book, &packet, None).unwrap();

    assert!(!report.accepted());
    assert!(
        report
            .obligations
            .iter()
            .any(|obligation| obligation.actual.contains("result-shape disagrees"))
    );
}

#[test]
fn woven_mode_never_asks_unconstrained_row() {
    let mut cx = cx();
    let packet = stamp_packet_cid(&weave_packet(vec![row(
        "budget",
        "reply",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]))
    .unwrap();
    let budget = first_weave(&packet);
    let fabric = RowFabric::new(vec![row(
        "answer",
        "reply",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]);

    let completed = weave_row_by_row(&mut cx, &fabric, packet, &budget).unwrap();
    let requests = fabric.requests();

    assert_eq!(requests.len(), 1);
    assert!(format!("{:?}", extra(&requests[0], "bridge-frontier-heads")).contains("reply"));
    assert!(format!("{:?}", extra(&requests[0], "bridge-role-refs")).contains("T1"));
    assert_eq!(
        extra(&requests[0], OUTPUT_GRAMMAR_REQUIRED_EXTRA),
        Some(&Expr::Bool(true))
    );
    assert!(
        matches!(extra(&requests[0], OUTPUT_GRAMMAR_EXTRA), Some(Expr::String(grammar)) if grammar.contains("\"head\"") && grammar.contains("\"const\""))
    );
    assert_eq!(
        extra(&requests[0], OUTPUT_GRAMMAR_DIALECT_EXTRA),
        Some(&Expr::Symbol(Symbol::new("json-schema")))
    );
    assert_eq!(first_weave(&completed).rows[0].slot, "answer");
}

#[test]
fn woven_and_validated_share_one_checker() {
    let mut cx = cx();
    let packet = stamp_packet_cid(&weave_packet(vec![row(
        "budget",
        "reply",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]))
    .unwrap();
    let bad = row(
        "answer",
        "invented-head",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    );
    let woven_report = validate_woven_row(&mut cx, &packet, &[], bad.clone()).unwrap();
    let validated_report =
        validate_weave(&mut cx, &packet, &BridgeWeavePayload::new(vec![bad])).unwrap();

    assert_eq!(woven_report.obligations, validated_report.obligations);
    assert!(
        woven_report
            .obligations
            .iter()
            .any(|obligation| obligation.path == "loom/rows/0/head")
    );
}

#[test]
fn repair_replaces_bad_row_by_path() {
    let mut cx = cx();
    let packet = stamp_packet_cid(&weave_packet(vec![row(
        "budget",
        "reply",
        vec![("input", Expr::Symbol(Symbol::new("T1")))],
    )]))
    .unwrap();
    let budget = first_weave(&packet);
    let fabric = RowFabric::new(vec![
        row(
            "bad",
            "invented-head",
            vec![("input", Expr::Symbol(Symbol::new("T1")))],
        ),
        row(
            "answer",
            "reply",
            vec![("input", Expr::Symbol(Symbol::new("T1")))],
        ),
    ]);

    let completed = weave_row_by_row(&mut cx, &fabric, packet, &budget).unwrap();
    let requests = fabric.requests();

    assert_eq!(requests.len(), 2);
    assert!(
        format!("{:?}", extra(&requests[1], "bridge-obligations")).contains("loom/rows/0/head")
    );
    assert_eq!(first_weave(&completed).rows[0].slot, "answer");
    assert_eq!(first_weave(&completed).rows[0].head, Symbol::new("reply"));
}
