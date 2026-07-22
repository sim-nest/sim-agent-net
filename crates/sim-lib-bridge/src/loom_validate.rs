use std::collections::BTreeSet;

use sim_codec_bridge::{BridgePacket, BridgeWeavePayload, validate_weave_payload};
use sim_kernel::{Cx, Expr, Result, Symbol};
use sim_value::access::field;

use crate::frontier::{FrontierMenu, frontier};
use crate::{BridgeObligation, BridgeReport};

/// One row-scoped LOOM validation defect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoomObligation {
    /// Stable row path where the defect was found.
    pub path: String,
    /// Human-facing reason.
    pub reason: String,
    /// Expected menu, Shape, reference, or capability.
    pub expected: String,
    /// Actual row value.
    pub actual: String,
    /// Valid replacements for the row position.
    pub valid_replacements: Vec<String>,
}

impl LoomObligation {
    /// Builds a row-scoped LOOM obligation.
    pub fn new(
        path: impl Into<String>,
        reason: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        valid_replacements: Vec<String>,
    ) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
            expected: expected.into(),
            actual: actual.into(),
            valid_replacements,
        }
    }

    /// Projects this LOOM obligation into the shared BRIDGE report format.
    pub fn to_bridge_obligation(&self) -> BridgeObligation {
        BridgeObligation::new(
            self.path.clone(),
            self.reason.clone(),
            self.expected.clone(),
            self.actual.clone(),
            self.valid_replacements.clone(),
        )
    }
}

/// Validates every `bridge/Weave` part in `packet`, appending row obligations
/// into `report`.
pub(crate) fn validate_packet_weaves(
    cx: &mut Cx,
    packet: &BridgePacket,
    report: &mut BridgeReport,
) -> Result<()> {
    for part in &packet.body {
        if part.kind != Symbol::qualified("bridge", "Weave") {
            continue;
        }
        let weave = match validate_weave_payload(&part.payload) {
            Ok(weave) => weave,
            Err(err) => {
                report.reject(&part.id);
                report.obligate(BridgeObligation::repair_packet(
                    format!("body/{}/payload", part.id.as_qualified_str()),
                    "invalid LOOM weave payload",
                    "derived result-shape agrees with rows",
                    err.to_string(),
                ));
                continue;
            }
        };
        let weave_report = validate_weave(cx, packet, &weave)?;
        if !weave_report.accepted() {
            report.reject(&part.id);
            for obligation in weave_report.obligations {
                report.obligate(obligation);
            }
        }
    }
    Ok(())
}

/// Validates one LOOM weave payload against the shared frontier.
pub fn validate_weave(
    cx: &mut Cx,
    packet: &BridgePacket,
    weave: &BridgeWeavePayload,
) -> Result<BridgeReport> {
    let mut report = BridgeReport::new(
        packet
            .header
            .cid
            .clone()
            .unwrap_or_else(|| "unstamped".to_owned()),
    );
    let mut available_refs = packet_refs(packet);
    let all_slots = weave
        .rows
        .iter()
        .map(|row| row.slot.clone())
        .collect::<BTreeSet<_>>();
    let mut defined_slots = BTreeSet::new();

    for (index, row) in weave.rows.iter().enumerate() {
        let row_path = format!("loom/rows/{index}");
        let menu = next_frontier_menu(cx, packet)?;
        let heads = head_choices(&menu);
        let head_names = heads
            .iter()
            .map(Symbol::as_qualified_str)
            .collect::<Vec<_>>();
        if !heads.contains(&row.head) {
            report.obligate(
                LoomObligation::new(
                    format!("{row_path}/head"),
                    "LOOM row selected an off-menu head",
                    head_names.join(", "),
                    row.head.as_qualified_str(),
                    head_names.clone(),
                )
                .to_bridge_obligation(),
            );
        }
        if !defined_slots.insert(row.slot.clone()) {
            report.obligate(
                LoomObligation::new(
                    format!("{row_path}/slot"),
                    "LOOM row redefines a slot",
                    "fresh row slot",
                    row.slot.clone(),
                    Vec::new(),
                )
                .to_bridge_obligation(),
            );
        }
        if let Some(required) = required_head_capability(&row.head)
            && !packet.header.ceiling.contains(&required)
        {
            report.obligate(
                LoomObligation::new(
                    format!("{row_path}/head"),
                    "LOOM row head requires a missing capability",
                    required.as_qualified_str(),
                    format!("{:?}", packet.header.ceiling),
                    vec![required.as_qualified_str()],
                )
                .to_bridge_obligation(),
            );
        }
        for (role, value) in &row.roles {
            check_role_ref(
                &mut report,
                &format!("{row_path}/roles/{}", role.as_qualified_str()),
                value,
                &available_refs,
                &all_slots,
            );
        }
        available_refs.insert(row.slot.clone());
    }

    Ok(report)
}

/// Computes the next LOOM frontier menu from the shared BRIDGE frontier engine.
pub fn next_frontier_menu(cx: &mut Cx, packet: &BridgePacket) -> Result<FrontierMenu> {
    frontier(cx, packet)
}

fn packet_refs(packet: &BridgePacket) -> BTreeSet<String> {
    packet
        .body
        .iter()
        .filter(|part| part.kind != Symbol::qualified("bridge", "Weave"))
        .map(|part| part.id.as_qualified_str())
        .chain(packet.header.context.iter().map(Symbol::as_qualified_str))
        .collect()
}

fn head_choices(menu: &FrontierMenu) -> Vec<Symbol> {
    match field(&menu.heads, "choices") {
        Some(Expr::Vector(items)) | Some(Expr::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Expr::Symbol(symbol) => Some(symbol.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn required_head_capability(head: &Symbol) -> Option<Symbol> {
    match head.name.as_ref() {
        "fetch" => Some(Symbol::qualified("bridge", "fetch")),
        "patch" => Some(Symbol::qualified("bridge", "patch")),
        _ => None,
    }
}

fn check_role_ref(
    report: &mut BridgeReport,
    path: &str,
    value: &Expr,
    available_refs: &BTreeSet<String>,
    all_slots: &BTreeSet<String>,
) {
    let Some(reference) = role_ref(value) else {
        report.obligate(
            LoomObligation::new(
                path,
                "LOOM role binding is not a reference",
                "symbol or @slot string",
                format!("{value:?}"),
                available_refs.iter().cloned().collect(),
            )
            .to_bridge_obligation(),
        );
        return;
    };
    if available_refs.contains(&reference) {
        return;
    }
    let reason = if all_slots.contains(&reference) {
        "LOOM role binding is a forward reference"
    } else {
        "LOOM role binding is dangling"
    };
    report.obligate(
        LoomObligation::new(
            path,
            reason,
            "previous row slot or packet context reference",
            reference,
            available_refs.iter().cloned().collect(),
        )
        .to_bridge_obligation(),
    );
}

fn role_ref(value: &Expr) -> Option<String> {
    match value {
        Expr::Symbol(symbol) => Some(symbol.as_qualified_str()),
        Expr::String(text) => text
            .strip_prefix('@')
            .filter(|slot| is_token(slot))
            .map(str::to_owned),
        _ => None,
    }
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii() && !ch.is_whitespace() && !matches!(ch, '{' | '}'))
}
