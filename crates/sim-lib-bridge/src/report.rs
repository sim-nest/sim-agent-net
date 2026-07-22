use sim_kernel::{Expr, Symbol};
use sim_value::build::entry;

/// One validation defect produced by BRIDGE receive checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeObligation {
    /// Stable packet path where the defect was found.
    pub path: String,
    /// Human-facing reason for the obligation.
    pub reason: String,
    /// Expected value, shape, or condition.
    pub expected: String,
    /// Actual value or condition observed.
    pub actual: String,
    /// Repair choices that a caller may offer to a human or model seat.
    pub repair_menu: Vec<String>,
}

impl BridgeObligation {
    /// Builds an obligation with explicit repair menu entries.
    pub fn new(
        path: impl Into<String>,
        reason: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        repair_menu: Vec<String>,
    ) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
            expected: expected.into(),
            actual: actual.into(),
            repair_menu,
        }
    }

    /// Builds an obligation with the default repair menu.
    pub fn repair_packet(
        path: impl Into<String>,
        reason: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::new(
            path,
            reason,
            expected,
            actual,
            vec![
                "repair packet".to_owned(),
                "fetch missing context".to_owned(),
                "return receipt".to_owned(),
            ],
        )
    }

    /// Projects this obligation to a stable expression record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            entry("path", Expr::String(self.path.clone())),
            entry("reason", Expr::String(self.reason.clone())),
            entry("expected", Expr::String(self.expected.clone())),
            entry("actual", Expr::String(self.actual.clone())),
            entry(
                "repair-menu",
                Expr::List(
                    self.repair_menu
                        .iter()
                        .map(|item| Expr::String(item.clone()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// Receive-check report for one BRIDGE packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeReport {
    /// Packet content id used for reporting.
    pub packet_cid: String,
    /// Accepted part ids.
    pub accepted_parts: Vec<String>,
    /// Rejected part ids.
    pub rejected_parts: Vec<String>,
    /// Obligations that must be repaired before the packet is accepted.
    pub obligations: Vec<BridgeObligation>,
}

impl BridgeReport {
    /// Builds an empty report for `packet_cid`.
    pub fn new(packet_cid: impl Into<String>) -> Self {
        Self {
            packet_cid: packet_cid.into(),
            accepted_parts: Vec::new(),
            rejected_parts: Vec::new(),
            obligations: Vec::new(),
        }
    }

    /// Records one accepted part id.
    pub fn accept(&mut self, id: &Symbol) {
        let id = id.as_qualified_str();
        if !self.accepted_parts.contains(&id) {
            self.accepted_parts.push(id);
        }
    }

    /// Records one rejected part id.
    pub fn reject(&mut self, id: &Symbol) {
        let id = id.as_qualified_str();
        if !self.rejected_parts.contains(&id) {
            self.rejected_parts.push(id);
        }
    }

    /// Appends an obligation.
    pub fn obligate(&mut self, obligation: BridgeObligation) {
        self.obligations.push(obligation);
    }

    /// Returns true when no obligation remains.
    pub fn accepted(&self) -> bool {
        self.obligations.is_empty()
    }

    /// Projects this report to a stable expression record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            entry("bridge-report", Expr::Bool(true)),
            entry("packet-cid", Expr::String(self.packet_cid.clone())),
            entry(
                "accepted-parts",
                Expr::List(
                    self.accepted_parts
                        .iter()
                        .map(|id| Expr::String(id.clone()))
                        .collect(),
                ),
            ),
            entry(
                "rejected-parts",
                Expr::List(
                    self.rejected_parts
                        .iter()
                        .map(|id| Expr::String(id.clone()))
                        .collect(),
                ),
            ),
            entry(
                "obligations",
                Expr::List(
                    self.obligations
                        .iter()
                        .map(BridgeObligation::to_expr)
                        .collect(),
                ),
            ),
        ])
    }
}
