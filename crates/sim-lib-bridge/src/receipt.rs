use sim_codec_bridge::{BridgeHeader, BridgePacket, BridgePart, BridgeProvenance};
use sim_kernel::{Expr, Result, Symbol};

use crate::report::BridgeReport;

/// Runtime symbol for the receipt function.
pub fn receipt_symbol() -> Symbol {
    Symbol::qualified("bridge", "receipt")
}

/// Builds a receipt packet carrying a report as a normal BRIDGE packet.
pub fn receipt_packet_for_report(
    report: &BridgeReport,
    from: impl Into<String>,
) -> Result<BridgePacket> {
    Ok(BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("receipt"),
            from: from.into(),
            to: Vec::new(),
            role: Symbol::new("checker"),
            parents: vec![report.packet_cid.clone()],
            task: Symbol::new("R1"),
            output: Symbol::new("R1"),
            ceiling: Vec::new(),
            context: Vec::new(),
            provenance: BridgeProvenance::default(),
        },
        body: vec![BridgePart {
            id: Symbol::new("R1"),
            kind: Symbol::qualified("bridge", "Receipt"),
            payload: Expr::Map(vec![sim_value::build::entry("report", report.to_expr())]),
        }],
        warrant: None,
    })
}
