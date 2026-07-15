use sim_codec_bridge::{
    BridgeBook, BridgeFramePayload, BridgeHeader, BridgePacket, BridgePart, BridgeProvenance,
    BridgeWeavePayload, BridgeWeaveRow, stamp_packet_cid,
};
use sim_kernel::{Expr, Symbol};
use sim_value::build::entry;

use crate::{next_frontier_menu, rx_check, validate_weave};

use super::cx;

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
