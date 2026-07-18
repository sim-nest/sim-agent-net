use std::collections::BTreeSet;
use std::sync::Arc;

use sim_codec_bridge::{
    BridgeBook, BridgePacket, BridgePart, BridgePartSpec, content_id_string, decode_bridge_text,
    expr_to_packet, packet_content_id,
};
use sim_kernel::{CapabilitySet, Cx, Diagnostic, Error, Expr, Result, Symbol, Value};
use sim_lib_agent_runner_core::{ModelResponse, effective_ceiling, terminal_model_content};
use sim_shape::{
    AnyShape, ExactExprShape, ExprKind, ExprKindShape, FieldShape, FieldSpec, MatchScore,
    OneOfShape, Shape, ShapeDoc, ShapeMatch, check_value_report, shape_value,
};
use sim_value::access::field;

use crate::loom_validate::validate_packet_weaves;
use crate::parent::parents_contain_cid;
use crate::report::{BridgeObligation, BridgeReport};
use crate::warrant::verify_warrant;

/// Resolves a packet's declared capability ceiling against the current context.
pub fn effective_caps(cx: &Cx, packet: &BridgePacket) -> Result<CapabilitySet> {
    effective_ceiling(cx.capabilities(), &packet.header.ceiling)
}

/// Checks a BRIDGE packet against the local book and optional parent packet.
///
/// A packet checked as a reply must cite the parent content id, send from one
/// of the parent's recipients, and address exactly the parent's sender. The
/// same check also validates move legality, part shapes, warrant freshness,
/// LOOM payloads, and the parent `Return` contract.
pub fn rx_check(
    cx: &mut Cx,
    book: &BridgeBook,
    packet: &BridgePacket,
    reply_to: Option<&BridgePacket>,
) -> Result<BridgeReport> {
    let mut report = BridgeReport::new(packet_report_cid(packet));
    check_header_linkage(&mut report, packet, reply_to);
    check_move(book, &mut report, packet, reply_to);
    check_parts(cx, book, &mut report, packet)?;
    for obligation in verify_warrant(cx, book, packet)? {
        report.obligate(obligation);
    }
    validate_packet_weaves(cx, packet, &mut report)?;
    if let Some(parent) = reply_to {
        check_parent_return(cx, &mut report, packet, parent)?;
    }
    Ok(report)
}

/// Decodes and checks the last content item of a model response.
pub fn bridge_rx_response(
    cx: &mut Cx,
    book: &BridgeBook,
    response: &ModelResponse,
    reply_to: Option<&BridgePacket>,
) -> Result<(BridgePacket, BridgeReport)> {
    let text = terminal_bridge_text(response)?;
    let packet = decode_bridge_text(text, book)?;
    let report = rx_check(cx, book, &packet, reply_to)?;
    if report.accepted() {
        Ok((packet, report))
    } else {
        Err(Error::Eval(format!(
            "bridge rx check failed: {:?}",
            report.obligations
        )))
    }
}

/// Decodes and checks a model response expression.
pub fn bridge_rx(
    cx: &mut Cx,
    book: &BridgeBook,
    response: Expr,
    reply_to: Option<&BridgePacket>,
) -> Result<(BridgePacket, BridgeReport)> {
    let response = ModelResponse::try_from(response)?;
    bridge_rx_response(cx, book, &response, reply_to)
}

pub(crate) fn terminal_bridge_text(response: &ModelResponse) -> Result<&str> {
    match terminal_model_content(response)? {
        Expr::String(text) => Ok(text),
        Expr::Map(_) => match field(terminal_model_content(response)?, "text") {
            Some(Expr::String(text)) => Ok(text),
            _ => Err(Error::Eval(
                "terminal BRIDGE response content must be a string or text map".to_owned(),
            )),
        },
        _ => Err(Error::Eval(
            "terminal BRIDGE response content must be text".to_owned(),
        )),
    }
}

fn packet_report_cid(packet: &BridgePacket) -> String {
    packet.header.cid.clone().unwrap_or_else(|| {
        packet_content_id(packet)
            .map(|id| content_id_string(&id))
            .unwrap_or_else(|_| "unhashable".to_owned())
    })
}

fn check_header_linkage(
    report: &mut BridgeReport,
    packet: &BridgePacket,
    reply_to: Option<&BridgePacket>,
) {
    let mut seen = BTreeSet::new();
    for part in &packet.body {
        if !seen.insert(part.id.clone()) {
            report.reject(&part.id);
            report.obligate(BridgeObligation::repair_packet(
                format!("body/{}", part.id.as_qualified_str()),
                "duplicate part id",
                "unique part id",
                part.id.as_qualified_str(),
            ));
        }
    }

    require_part_id(report, packet, &packet.header.task, "header/task");
    require_part_id(report, packet, &packet.header.output, "header/output");
    for context in &packet.header.context {
        require_part_id(report, packet, context, "header/context");
    }

    if let Some(parent) = reply_to {
        let Some(parent_cid) = &parent.header.cid else {
            report.obligate(BridgeObligation::repair_packet(
                "header/parents",
                "parent packet is missing a cid",
                "stamped parent cid",
                "none",
            ));
            return;
        };
        if !parents_contain_cid(&packet.header.parents, parent_cid) {
            report.obligate(BridgeObligation::repair_packet(
                "header/parents",
                "reply does not cite parent packet",
                parent_cid,
                format!("{:?}", packet.header.parents),
            ));
        }
        check_reply_actor_linkage(report, packet, parent);
    }
}

fn check_reply_actor_linkage(
    report: &mut BridgeReport,
    packet: &BridgePacket,
    parent: &BridgePacket,
) {
    if !parent
        .header
        .to
        .iter()
        .any(|actor| actor == &packet.header.from)
    {
        let expected = if parent.header.to.is_empty() {
            "one parent header/to recipient".to_owned()
        } else {
            format!("{:?}", parent.header.to)
        };
        report.obligate(BridgeObligation::repair_packet(
            "header/from",
            "reply sender is not a parent recipient",
            expected,
            packet.header.from.clone(),
        ));
    }

    let expected_to = vec![parent.header.from.clone()];
    if packet.header.to != expected_to {
        report.obligate(BridgeObligation::repair_packet(
            "header/to",
            "reply recipient does not mirror parent sender",
            format!("{expected_to:?}"),
            format!("{:?}", packet.header.to),
        ));
    }
}

fn require_part_id(report: &mut BridgeReport, packet: &BridgePacket, id: &Symbol, path: &str) {
    if packet.body.iter().any(|part| &part.id == id) {
        return;
    }
    report.obligate(BridgeObligation::repair_packet(
        path,
        "header references a missing part",
        id.as_qualified_str(),
        "missing",
    ));
}

fn check_move(
    book: &BridgeBook,
    report: &mut BridgeReport,
    packet: &BridgePacket,
    reply_to: Option<&BridgePacket>,
) {
    let parent_moves = reply_to
        .map(|parent| vec![parent.header.move_kind.clone()])
        .unwrap_or_default();
    let part_kinds = packet
        .body
        .iter()
        .map(|part| part.kind.clone())
        .collect::<Vec<_>>();
    if let Err(err) = book
        .moves
        .check_move(&packet.header.move_kind, &parent_moves, &part_kinds)
    {
        report.obligate(BridgeObligation::repair_packet(
            "header/move",
            "illegal BRIDGE move",
            "move book accepts intent, parent, and required parts",
            err.to_string(),
        ));
    }
}

fn check_parts(
    cx: &mut Cx,
    book: &BridgeBook,
    report: &mut BridgeReport,
    packet: &BridgePacket,
) -> Result<()> {
    for part in &packet.body {
        match book.parts.spec(&part.kind) {
            Some(spec) => {
                if check_part_shape(cx, spec, part, report)? {
                    report.accept(&part.id);
                } else {
                    report.reject(&part.id);
                }
            }
            None => {
                report.reject(&part.id);
                report.obligate(BridgeObligation::repair_packet(
                    format!("body/{}", part.id.as_qualified_str()),
                    "unknown BRIDGE part kind",
                    "registered part kind",
                    part.kind.as_qualified_str(),
                ));
            }
        }
    }
    Ok(())
}

fn check_part_shape(
    cx: &mut Cx,
    spec: &BridgePartSpec,
    part: &BridgePart,
    report: &mut BridgeReport,
) -> Result<bool> {
    let expected = expected_part_payload_shape(spec);
    check_payload_shape(
        cx,
        &format!("body/{}/payload", part.id.as_qualified_str()),
        &format!("{} payload", spec.kind.as_qualified_str()),
        expected,
        &part.payload,
        report,
    )
}

fn expected_part_payload_shape(spec: &BridgePartSpec) -> Arc<dyn Shape> {
    if spec.kind == Symbol::qualified("bridge", "Extension")
        || spec.kind == Symbol::qualified("bridge", "Return")
    {
        Arc::new(AnyShape)
    } else {
        Arc::new(ExprKindShape::new(ExprKind::Map))
    }
}

fn check_parent_return(
    cx: &mut Cx,
    report: &mut BridgeReport,
    packet: &BridgePacket,
    parent: &BridgePacket,
) -> Result<()> {
    let Some(contract) = part_by_id(parent, &parent.header.output) else {
        report.obligate(BridgeObligation::repair_packet(
            "reply-to/header/output",
            "parent output part is missing",
            parent.header.output.as_qualified_str(),
            "missing",
        ));
        return Ok(());
    };
    if contract.kind != Symbol::qualified("bridge", "Return") {
        report.obligate(BridgeObligation::repair_packet(
            "reply-to/header/output",
            "parent output part is not a Return contract",
            "bridge/Return",
            contract.kind.as_qualified_str(),
        ));
        return Ok(());
    }

    let Some(shape_expr) = field(&contract.payload, "shape") else {
        return Ok(());
    };
    let Some(reply_output) = part_by_id(packet, &packet.header.output) else {
        report.obligate(BridgeObligation::repair_packet(
            "header/output",
            "reply output part is missing",
            packet.header.output.as_qualified_str(),
            "missing",
        ));
        return Ok(());
    };
    let Some(shape) = shape_from_contract_expr(shape_expr) else {
        report.obligate(BridgeObligation::repair_packet(
            "reply-to/return/shape",
            "unsupported Return shape expression",
            "core primitives, bridge/Answer, bridge/Refusal, or shape/OneOf",
            format!("{shape_expr:?}"),
        ));
        return Ok(());
    };
    check_payload_shape(
        cx,
        &format!("body/{}/payload", reply_output.id.as_qualified_str()),
        "parent Return contract",
        shape,
        &reply_output.payload,
        report,
    )?;
    Ok(())
}

fn part_by_id<'a>(packet: &'a BridgePacket, id: &Symbol) -> Option<&'a BridgePart> {
    packet.body.iter().find(|part| &part.id == id)
}

pub(crate) fn shape_from_contract_expr(expr: &Expr) -> Option<Arc<dyn Shape>> {
    match expr {
        Expr::Symbol(symbol) => symbol_shape(symbol),
        Expr::Map(_) => shape_descriptor(expr),
        _ => None,
    }
}

fn symbol_shape(symbol: &Symbol) -> Option<Arc<dyn Shape>> {
    let shape: Arc<dyn Shape> = match (symbol.namespace.as_deref(), symbol.name.as_ref()) {
        (Some("core"), "Any") => Arc::new(AnyShape),
        (Some("core"), "String") | (Some("bridge"), "Answer") => {
            Arc::new(ExprKindShape::new(ExprKind::String))
        }
        (Some("core"), "Bool") => Arc::new(ExprKindShape::new(ExprKind::Bool)),
        (Some("core"), "Number") => Arc::new(ExprKindShape::new(ExprKind::Number)),
        (Some("core"), "Symbol") => Arc::new(ExprKindShape::new(ExprKind::Symbol)),
        (Some("core"), "List") => Arc::new(ExprKindShape::new(ExprKind::List)),
        (Some("core"), "Map") => Arc::new(ExprKindShape::new(ExprKind::Map)),
        (Some("bridge"), "Packet") => Arc::new(BridgePacketExprShape),
        (Some("bridge"), "Refusal") => refusal_shape(),
        _ => return None,
    };
    Some(shape)
}

struct BridgePacketExprShape;

impl Shape for BridgePacketExprShape {
    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol::qualified("bridge", "Packet"))
    }

    fn check_value(&self, cx: &mut Cx, value: Value) -> Result<ShapeMatch> {
        let expr = value.object().as_expr(cx)?;
        self.check_expr(cx, &expr)
    }

    fn check_expr(&self, _cx: &mut Cx, expr: &Expr) -> Result<ShapeMatch> {
        match expr_to_packet(expr) {
            Ok(_) => Ok(ShapeMatch::accept(MatchScore::exact(10))),
            Err(err) => Ok(ShapeMatch::reject_with_diagnostic(Diagnostic::error(
                format!("expected bridge/Packet expression: {err}"),
            ))),
        }
    }

    fn describe(&self, _cx: &mut Cx) -> Result<ShapeDoc> {
        Ok(ShapeDoc::new("bridge/Packet"))
    }
}

fn shape_descriptor(expr: &Expr) -> Option<Arc<dyn Shape>> {
    let Some(Expr::Symbol(kind)) = field(expr, "shape") else {
        return None;
    };
    if kind != &Symbol::qualified("shape", "OneOf") {
        return None;
    }
    let choices = match field(expr, "choices")? {
        Expr::Vector(items) | Expr::List(items) => items,
        _ => return None,
    };
    let choices = choices
        .iter()
        .map(shape_from_contract_expr)
        .collect::<Option<Vec<_>>>()?;
    Some(Arc::new(OneOfShape::new(choices)))
}

fn refusal_shape() -> Arc<dyn Shape> {
    Arc::new(FieldShape::anonymous(vec![
        FieldSpec::required(
            Symbol::new("kind"),
            Arc::new(ExactExprShape::new(Expr::Symbol(Symbol::qualified(
                "bridge", "Refusal",
            )))),
        ),
        FieldSpec::required(
            Symbol::new("reason"),
            Arc::new(ExprKindShape::new(ExprKind::String)),
        ),
    ]))
}

fn check_payload_shape(
    cx: &mut Cx,
    path: &str,
    expected: &str,
    shape: Arc<dyn Shape>,
    payload: &Expr,
    report: &mut BridgeReport,
) -> Result<bool> {
    let shape_ref = shape_value(Symbol::qualified("bridge", expected), shape);
    let value = cx.factory().expr(payload.clone())?;
    let matched = check_value_report(cx, &shape_ref, value)?;
    if matched.accepted {
        return Ok(true);
    }
    report.obligate(BridgeObligation::repair_packet(
        path,
        "payload failed Shape check",
        expected,
        if matched.diagnostics.is_empty() {
            format!("{payload:?}")
        } else {
            matched
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        },
    ));
    Ok(false)
}
