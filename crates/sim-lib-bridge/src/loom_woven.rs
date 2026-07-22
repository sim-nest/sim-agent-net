use sim_codec_bridge::{
    BridgeBook, BridgePacket, BridgePart, BridgeWeavePayload, BridgeWeaveRow, WeavePart,
    stamp_packet_cid,
};
use sim_kernel::{
    Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Expr, Result, Symbol,
};
use sim_lib_agent_runner_core::{
    ModelRequest, ModelResponse, OUTPUT_GRAMMAR_DIALECT_EXTRA, OUTPUT_GRAMMAR_EXTRA,
    OUTPUT_GRAMMAR_REQUIRED_EXTRA, RETURN_CODEC_EXTRA, RETURN_SHAPE_EXTRA, shape_to_grammar,
    terminal_model_content,
};
use sim_shape::{ExprKind, ExprKindShape, FieldShape, FieldSpec};
use sim_value::{access::field, build::entry};
use std::sync::Arc;

use crate::frontier::FrontierMenu;
use crate::loom_validate::{next_frontier_menu, validate_weave};
use crate::report::{BridgeObligation, BridgeReport};
use crate::rx::{effective_caps, rx_check};

/// Validates one candidate woven row by appending it to the accepted prefix and
/// running the shared LOOM checker.
pub fn validate_woven_row(
    cx: &mut Cx,
    packet: &BridgePacket,
    accepted_rows: &[BridgeWeaveRow],
    row: BridgeWeaveRow,
) -> Result<BridgeReport> {
    let rows = accepted_rows
        .iter()
        .cloned()
        .chain(std::iter::once(row))
        .collect();
    let weave = BridgeWeavePayload::new(rows);
    validate_weave(cx, &packet_with_weave(packet, &weave)?, &weave)
}

/// Runs LOOM woven mode, requesting and committing one row at a time.
///
/// The `weave` argument supplies the row budget for this run. Each requested
/// row is decoded as a single [`BridgeWeaveRow`], checked through the same
/// validator used by validated mode, and committed before the next request is
/// built. A rejected row receives one constrained replacement request carrying
/// the exact row path, current frontier, role-reference menu, and obligations.
pub fn weave_row_by_row(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    packet: BridgePacket,
    weave: &WeavePart,
) -> Result<BridgePacket> {
    if weave.rows.is_empty() {
        return Err(Error::Eval(
            "LOOM woven mode requires at least one row budget entry".to_owned(),
        ));
    }
    require_weave_part(&packet)?;

    let mut accepted_rows = Vec::new();
    for row_index in 0..weave.rows.len() {
        let row = request_woven_row(cx, target, &packet, &accepted_rows, row_index, &[])?;
        let report = validate_woven_row(cx, &packet, &accepted_rows, row.clone())?;
        if report.accepted() {
            accepted_rows.push(row);
            continue;
        }

        let replacement = request_woven_row(
            cx,
            target,
            &packet,
            &accepted_rows,
            row_index,
            &report.obligations,
        )?;
        let replacement_report =
            validate_woven_row(cx, &packet, &accepted_rows, replacement.clone())?;
        if !replacement_report.accepted() {
            return Err(Error::Eval(format!(
                "LOOM woven repair failed at loom/rows/{row_index}: {:?}",
                replacement_report.obligations
            )));
        }
        accepted_rows.push(replacement);
    }

    let mut completed = packet_with_weave(&packet, &BridgeWeavePayload::new(accepted_rows))?;
    completed.header.cid = None;
    let completed = stamp_packet_cid(&completed)?;
    let book = BridgeBook::standard();
    let report = rx_check(cx, &book, &completed, None)?;
    if !report.accepted() {
        return Err(Error::Eval(format!(
            "LOOM woven packet failed final check: {:?}",
            report.obligations
        )));
    }
    Ok(completed)
}

fn request_woven_row(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    packet: &BridgePacket,
    accepted_rows: &[BridgeWeaveRow],
    row_index: usize,
    obligations: &[BridgeObligation],
) -> Result<BridgeWeaveRow> {
    let request = woven_row_request(cx, packet, accepted_rows, row_index, obligations)?;
    let caps = effective_caps(cx, packet)?;
    let reply = cx.with_capabilities(caps, |cx| target.realize(cx, request))?;
    decode_row_reply(cx, reply)
}

fn woven_row_request(
    cx: &mut Cx,
    packet: &BridgePacket,
    accepted_rows: &[BridgeWeaveRow],
    row_index: usize,
    obligations: &[BridgeObligation],
) -> Result<EvalRequest> {
    let partial_weave = BridgeWeavePayload::new(accepted_rows.to_vec());
    let partial_packet = packet_with_optional_weave(packet, Some(&partial_weave))?;
    let menu = next_frontier_menu(cx, &partial_packet)?;
    let role_refs = role_ref_menu(packet, accepted_rows);
    let mut model = ModelRequest::new(
        Expr::Map(vec![
            entry(
                "mode",
                Expr::Symbol(Symbol::qualified("bridge", "loom-woven-row")),
            ),
            entry("row-path", row_path(row_index)),
            entry("head-menu", menu.heads.clone()),
            entry("role-ref-menu", string_vector(&role_refs)),
            entry("slot-menu", slot_menu_expr(&menu)),
            entry("obligations", obligations_expr(obligations)),
        ]),
        Vec::new(),
    );
    model.extra.push(entry(
        "bridge-mode",
        Expr::Symbol(Symbol::qualified("bridge", "loom-woven-row")),
    ));
    model
        .extra
        .push(entry("bridge-row-path", row_path(row_index)));
    model
        .extra
        .push(entry("bridge-frontier-heads", menu.heads.clone()));
    model
        .extra
        .push(entry("bridge-frontier-slots", slot_menu_expr(&menu)));
    model
        .extra
        .push(entry("bridge-role-refs", string_vector(&role_refs)));
    model.extra.push(entry(
        "bridge-frontier-grammar",
        Expr::String(menu.grammar.clone()),
    ));
    if !obligations.is_empty() {
        model
            .extra
            .push(entry("bridge-obligations", obligations_expr(obligations)));
    }
    model.extra.push(entry(
        RETURN_CODEC_EXTRA,
        Expr::Symbol(Symbol::qualified("codec", "bridge")),
    ));
    model
        .extra
        .push(entry(RETURN_SHAPE_EXTRA, row_shape_expr(&menu, &role_refs)));
    model.extra.push(entry(
        OUTPUT_GRAMMAR_EXTRA,
        Expr::String(row_grammar(&menu)?),
    ));
    model.extra.push(entry(
        OUTPUT_GRAMMAR_DIALECT_EXTRA,
        Expr::Symbol(Symbol::new("json-schema")),
    ));
    model
        .extra
        .push(entry(OUTPUT_GRAMMAR_REQUIRED_EXTRA, Expr::Bool(true)));

    Ok(EvalRequest {
        expr: Expr::from(model),
        result_shape: None,
        required_capabilities: effective_caps(cx, packet)?.iter().cloned().collect(),
        deadline: None,
        consistency: Consistency::default(),
        mode: EvalMode::default(),
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    })
}

fn decode_row_reply(cx: &mut Cx, reply: EvalReply) -> Result<BridgeWeaveRow> {
    let response = ModelResponse::try_from(reply.value.object().as_expr(cx)?)?;
    let content = terminal_model_content(&response)?;
    BridgeWeaveRow::from_expr(field(content, "row").unwrap_or(content))
}

fn packet_with_weave(packet: &BridgePacket, weave: &BridgeWeavePayload) -> Result<BridgePacket> {
    packet_with_optional_weave(packet, Some(weave))
}

fn packet_with_optional_weave(
    packet: &BridgePacket,
    weave: Option<&BridgeWeavePayload>,
) -> Result<BridgePacket> {
    let mut packet = packet.clone();
    let index = require_weave_part(&packet)?;
    if let Some(weave) = weave {
        packet.body[index].payload = weave.to_expr();
    }
    packet.header.cid = None;
    Ok(packet)
}

fn require_weave_part(packet: &BridgePacket) -> Result<usize> {
    packet
        .body
        .iter()
        .position(is_weave_part)
        .ok_or_else(|| Error::Eval("LOOM woven mode requires a bridge/Weave part".to_owned()))
}

fn is_weave_part(part: &BridgePart) -> bool {
    part.kind == Symbol::qualified("bridge", "Weave")
}

fn role_ref_menu(packet: &BridgePacket, accepted_rows: &[BridgeWeaveRow]) -> Vec<String> {
    let mut refs = packet
        .body
        .iter()
        .filter(|part| !is_weave_part(part))
        .map(|part| part.id.as_qualified_str())
        .chain(packet.header.context.iter().map(Symbol::as_qualified_str))
        .chain(accepted_rows.iter().map(|row| row.slot.clone()))
        .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

fn row_shape_expr(menu: &FrontierMenu, role_refs: &[String]) -> Expr {
    Expr::Map(vec![
        entry(
            "shape",
            Expr::Symbol(Symbol::qualified("bridge", "WeaveRow")),
        ),
        entry("slot", Expr::Symbol(Symbol::qualified("core", "String"))),
        entry("head", menu.heads.clone()),
        entry("roles", string_vector(role_refs)),
    ])
}

fn row_grammar(menu: &FrontierMenu) -> Result<String> {
    let shape = FieldShape::anonymous(vec![
        FieldSpec::required(
            Symbol::new("slot"),
            Arc::new(ExprKindShape::new(ExprKind::String)),
        ),
        FieldSpec::required(Symbol::new("head"), menu.head_shape.clone()),
        FieldSpec::required(
            Symbol::new("roles"),
            Arc::new(ExprKindShape::new(ExprKind::Map)),
        ),
    ]);
    shape_to_grammar(&shape)
}

fn slot_menu_expr(menu: &FrontierMenu) -> Expr {
    Expr::Vector(
        menu.slots
            .iter()
            .map(|(slot, shape)| {
                Expr::Map(vec![
                    entry("slot", Expr::String(slot.clone())),
                    entry("shape", shape.clone()),
                ])
            })
            .collect(),
    )
}

fn obligations_expr(obligations: &[BridgeObligation]) -> Expr {
    Expr::Vector(obligations.iter().map(BridgeObligation::to_expr).collect())
}

fn string_vector(items: &[String]) -> Expr {
    Expr::Vector(items.iter().cloned().map(Expr::String).collect())
}

fn row_path(row_index: usize) -> Expr {
    Expr::String(format!("loom/rows/{row_index}"))
}
