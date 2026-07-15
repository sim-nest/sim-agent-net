use sim_codec_bridge::{
    BridgeBook, BridgeHeader, BridgePacket, BridgePart, BridgeProvenance, content_id_string,
    packet_content_id, packet_to_expr, stamp_packet_cid,
};
use sim_kernel::{
    Consistency, ContentId, Cx, Error, EvalFabric, EvalMode, EvalRequest, Expr, Result, Symbol,
};
use sim_lib_agent_runner_core::{
    ModelRequest, ModelResponse, OUTPUT_GRAMMAR_EXTRA, OUTPUT_GRAMMAR_REQUIRED_EXTRA,
    RETURN_CODEC_EXTRA, RETURN_SHAPE_EXTRA, terminal_model_content,
};
use sim_lib_bridge::{
    BridgeObligation, BridgeReport, FrontierMenu, effective_caps, frontier, rx_check,
};
use sim_value::{access::field, build::entry};

use crate::lift::{compiled_intent, report_summary, validate_candidate};
use crate::normalize::normalize_prose;
use crate::{CompiledIntent, LiftOptions};

const FRONTIER_TARGET: &str = "model:forge-frontier";
const MAX_FRONTIER_ATTEMPTS: usize = 16;

/// Compiles prose into a checked candidate packet by authoring one BRIDGE part
/// at a time through the shared frontier menu.
pub fn forge_lift_frontier(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    prose: &str,
    opts: &LiftOptions,
) -> Result<CompiledIntent> {
    let (normalized, source) = normalize_prose(prose)?;
    let book = BridgeBook::standard();
    let mut packet = seed_packet(&normalized, &source)?;
    let mut row_index = 0usize;
    let mut obligations = Vec::new();

    for _ in 0..MAX_FRONTIER_ATTEMPTS {
        if let Some(intent) = completed_intent(cx, &book, opts, source.clone(), &packet)? {
            return Ok(intent);
        }

        let expected = expected_next_part(&packet)?;
        let menu = frontier(cx, &packet)?;
        let row = request_frontier_part(
            cx,
            target,
            &packet,
            &menu,
            &expected,
            row_index,
            &obligations,
        )?;
        let report = validate_frontier_part(cx, &book, &packet, &menu, &expected, row_index, &row)?;
        if !report.accepted() {
            obligations = report.obligations;
            continue;
        }

        packet.body.push(row.part);
        packet.header.cid = None;
        obligations.clear();
        row_index += 1;
    }

    Err(Error::Eval(format!(
        "forge frontier lift failed after {MAX_FRONTIER_ATTEMPTS} attempt(s): {}",
        report_summary(&BridgeReport {
            packet_cid: packet
                .header
                .cid
                .clone()
                .unwrap_or_else(|| "unstamped".to_owned()),
            accepted_parts: Vec::new(),
            rejected_parts: Vec::new(),
            obligations,
        })
    )))
}

fn seed_packet(normalized: &str, source: &ContentId) -> Result<BridgePacket> {
    Ok(BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec![FRONTIER_TARGET.to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("T1"),
            output: Symbol::new("O1"),
            ceiling: vec![Symbol::qualified("ai", "run")],
            context: vec![Symbol::new("G1")],
            provenance: BridgeProvenance::default(),
        },
        body: vec![BridgePart {
            id: Symbol::new("G1"),
            kind: Symbol::qualified("bridge", "Given"),
            payload: Expr::Map(vec![
                entry(
                    "kind",
                    Expr::Symbol(Symbol::qualified("forge", "ProseSource")),
                ),
                entry("content-id", Expr::String(content_id_string(source))),
                entry("prose", Expr::String(normalized.to_owned())),
            ]),
        }],
        warrant: None,
    })
}

fn completed_intent(
    cx: &mut Cx,
    book: &BridgeBook,
    opts: &LiftOptions,
    source: ContentId,
    packet: &BridgePacket,
) -> Result<Option<CompiledIntent>> {
    if !packet.body.iter().any(|part| part.id == packet.header.task)
        || !packet
            .body
            .iter()
            .any(|part| part.id == packet.header.output)
    {
        return Ok(None);
    }
    let stamped = stamp_packet_cid(packet)?;
    let report = validate_candidate(cx, book, &stamped)?;
    if report.accepted() {
        Ok(Some(compiled_intent(opts, source, stamped)?))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedPart {
    TaskFrame,
    Return,
}

impl ExpectedPart {
    fn id(self) -> Symbol {
        match self {
            Self::TaskFrame => Symbol::new("T1"),
            Self::Return => Symbol::new("O1"),
        }
    }

    fn kind(self) -> Symbol {
        match self {
            Self::TaskFrame => Symbol::qualified("bridge", "Frame"),
            Self::Return => Symbol::qualified("bridge", "Return"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::TaskFrame => "task Frame",
            Self::Return => "Return",
        }
    }

    fn to_expr(self) -> Expr {
        Expr::Map(vec![
            entry("id", Expr::Symbol(self.id())),
            entry("kind", Expr::Symbol(self.kind())),
            entry("label", Expr::String(self.label().to_owned())),
        ])
    }
}

fn expected_next_part(packet: &BridgePacket) -> Result<ExpectedPart> {
    if !packet.body.iter().any(|part| part.id == packet.header.task) {
        return Ok(ExpectedPart::TaskFrame);
    }
    if !packet
        .body
        .iter()
        .any(|part| part.id == packet.header.output)
    {
        return Ok(ExpectedPart::Return);
    }
    Err(Error::Eval(
        "forge frontier packet has task and return but is still incomplete".to_owned(),
    ))
}

struct FrontierPartRow {
    head: Symbol,
    part: BridgePart,
}

fn request_frontier_part(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    packet: &BridgePacket,
    menu: &FrontierMenu,
    expected: &ExpectedPart,
    row_index: usize,
    obligations: &[BridgeObligation],
) -> Result<FrontierPartRow> {
    let request = frontier_part_request(cx, packet, menu, expected, row_index, obligations)?;
    let reply = target.realize(cx, request)?;
    decode_frontier_part_reply(cx, reply)
}

fn frontier_part_request(
    cx: &mut Cx,
    packet: &BridgePacket,
    menu: &FrontierMenu,
    expected: &ExpectedPart,
    row_index: usize,
    obligations: &[BridgeObligation],
) -> Result<EvalRequest> {
    let row_path = format!("frontier/rows/{row_index}");
    let mut model = ModelRequest::new(
        Expr::Map(vec![
            entry(
                "mode",
                Expr::Symbol(Symbol::qualified("forge", "frontier-part")),
            ),
            entry("row-path", Expr::String(row_path.clone())),
            entry("expected-part", expected.to_expr()),
            entry("head-menu", menu.heads.clone()),
            entry("slot-menu", slot_menu_expr(menu)),
            entry("partial-packet", packet_to_expr(packet)),
            entry("obligations", obligations_expr(obligations)),
        ]),
        Vec::new(),
    );
    model.extra.push(entry(
        "forge-mode",
        Expr::Symbol(Symbol::qualified("forge", "frontier-part")),
    ));
    model
        .extra
        .push(entry("forge-row-path", Expr::String(row_path)));
    model
        .extra
        .push(entry("forge-expected-part", expected.to_expr()));
    model
        .extra
        .push(entry("forge-frontier-heads", menu.heads.clone()));
    model
        .extra
        .push(entry("forge-frontier-slots", slot_menu_expr(menu)));
    model.extra.push(entry(
        "forge-frontier-grammar",
        Expr::String(menu.grammar.clone()),
    ));
    if !obligations.is_empty() {
        model
            .extra
            .push(entry("forge-obligations", obligations_expr(obligations)));
    }
    model.extra.push(entry(
        RETURN_CODEC_EXTRA,
        Expr::Symbol(Symbol::qualified("codec", "json")),
    ));
    model
        .extra
        .push(entry(RETURN_SHAPE_EXTRA, row_shape_expr(menu, expected)));
    model
        .extra
        .push(entry(OUTPUT_GRAMMAR_EXTRA, Expr::String(row_grammar(menu))));
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

fn decode_frontier_part_reply(
    cx: &mut Cx,
    reply: sim_kernel::EvalReply,
) -> Result<FrontierPartRow> {
    let response = ModelResponse::try_from(reply.value.object().as_expr(cx)?)?;
    let content = terminal_model_content(&response)?;
    let expr = field(content, "row").unwrap_or(content);
    let head = match field(expr, "head") {
        Some(Expr::Symbol(symbol)) => symbol.clone(),
        Some(Expr::String(text)) => Symbol::new(text.as_str()),
        _ => {
            return Err(Error::Eval(
                "forge frontier row is missing symbolic head".to_owned(),
            ));
        }
    };
    let part = part_from_expr(
        field(expr, "part")
            .ok_or_else(|| Error::Eval("forge frontier row is missing part".to_owned()))?,
    )?;
    Ok(FrontierPartRow { head, part })
}

fn part_from_expr(expr: &Expr) -> Result<BridgePart> {
    let id = match field(expr, "id") {
        Some(Expr::Symbol(symbol)) => symbol.clone(),
        _ => return Err(Error::Eval("forge frontier part is missing id".to_owned())),
    };
    let kind = match field(expr, "kind") {
        Some(Expr::Symbol(symbol)) => symbol.clone(),
        _ => {
            return Err(Error::Eval(
                "forge frontier part is missing kind".to_owned(),
            ));
        }
    };
    let payload = field(expr, "payload")
        .ok_or_else(|| Error::Eval("forge frontier part is missing payload".to_owned()))?
        .clone();
    Ok(BridgePart { id, kind, payload })
}

fn validate_frontier_part(
    cx: &mut Cx,
    book: &BridgeBook,
    packet: &BridgePacket,
    menu: &FrontierMenu,
    expected: &ExpectedPart,
    row_index: usize,
    row: &FrontierPartRow,
) -> Result<BridgeReport> {
    let mut report = BridgeReport::new(packet_report_cid(packet));
    let head_choices = head_choices(menu);
    if !head_choices.contains(&row.head) {
        report.obligate(BridgeObligation::new(
            format!("frontier/rows/{row_index}/head"),
            "frontier row selected an off-menu head",
            head_choices
                .iter()
                .map(Symbol::as_qualified_str)
                .collect::<Vec<_>>()
                .join(", "),
            row.head.as_qualified_str(),
            head_choices
                .iter()
                .map(Symbol::as_qualified_str)
                .collect::<Vec<_>>(),
        ));
        return Ok(report);
    }

    if row.part.id != expected.id() || row.part.kind != expected.kind() {
        report.obligate(BridgeObligation::new(
            format!("frontier/rows/{row_index}/part"),
            "frontier row supplied the wrong part slot",
            format!("{} {}", expected.id(), expected.kind()),
            format!("{} {}", row.part.id, row.part.kind),
            vec![expected.label().to_owned()],
        ));
        return Ok(report);
    }

    if matches!(expected, ExpectedPart::TaskFrame)
        && let Err(err) = book.frames.validate_payload(&row.part.payload)
    {
        report.obligate(BridgeObligation::new(
            format!("frontier/rows/{row_index}/part"),
            "frontier row part did not satisfy the expected Shape",
            "registered bridge/Frame payload",
            err.to_string(),
            vec![expected.label().to_owned()],
        ));
        return Ok(report);
    }

    let mut candidate = packet.clone();
    candidate.body.push(row.part.clone());
    candidate.header.cid = None;
    let candidate_report = if matches!(expected, ExpectedPart::Return) {
        validate_candidate(cx, book, &candidate)?
    } else {
        rx_check(cx, book, &candidate, None)?
    };
    let part_id = row.part.id.as_qualified_str();
    let part_path = format!("body/{part_id}");
    let part_obligations = candidate_report
        .obligations
        .iter()
        .filter(|obligation| obligation.path.starts_with(&part_path))
        .cloned()
        .collect::<Vec<_>>();
    if candidate_report.accepted_parts.contains(&part_id) && part_obligations.is_empty() {
        report.accept(&row.part.id);
    } else {
        report.reject(&row.part.id);
        if part_obligations.is_empty() {
            report.obligate(BridgeObligation::new(
                format!("frontier/rows/{row_index}/part"),
                "frontier row part did not satisfy the expected Shape",
                expected.label(),
                format!("{} {}", row.part.id, row.part.kind),
                vec![expected.label().to_owned()],
            ));
        } else {
            for obligation in part_obligations {
                report.obligate(row_obligation(row_index, obligation));
            }
        }
    }
    Ok(report)
}

fn packet_report_cid(packet: &BridgePacket) -> String {
    packet
        .header
        .cid
        .clone()
        .unwrap_or_else(|| match packet_content_id(packet) {
            Ok(id) => content_id_string(&id),
            Err(_) => "unhashable".to_owned(),
        })
}

fn row_obligation(row_index: usize, obligation: BridgeObligation) -> BridgeObligation {
    BridgeObligation::new(
        format!("frontier/rows/{row_index}/part"),
        obligation.reason,
        obligation.expected,
        obligation.actual,
        obligation.repair_menu,
    )
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

fn row_shape_expr(menu: &FrontierMenu, expected: &ExpectedPart) -> Expr {
    Expr::Map(vec![
        entry(
            "shape",
            Expr::Symbol(Symbol::qualified("forge", "FrontierPart")),
        ),
        entry("head", menu.heads.clone()),
        entry("expected-part", expected.to_expr()),
    ])
}

fn row_grammar(menu: &FrontierMenu) -> String {
    format!(
        "{{\"type\":\"object\",\"required\":[\"head\",\"part\"],\"properties\":{{\"head\":{},\"part\":{{\"type\":\"object\",\"required\":[\"id\",\"kind\",\"payload\"]}}}}}}",
        menu.grammar
    )
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
