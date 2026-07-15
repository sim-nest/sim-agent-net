use sim_codec_bridge::{
    BridgeBook, BridgeCallArgument, BridgeCallPayload, BridgeHeader, BridgePacket, BridgePart,
    BridgeProvenance, CallArgumentMedia, content_id_string, expr_to_packet, packet_content_id,
    stamp_packet_cid,
};
use sim_kernel::{ContentId, Cx, Datum, Error, EvalFabric, Expr, Result, Symbol};
use sim_lib_agent_runner_core::InjectionFence;
use sim_lib_bridge::{BridgeObligation, BridgeReport, RepairPolicy, run_bridge, rx_check};
use sim_value::build::entry;

use crate::normalize::normalize_prose;
use crate::{CompiledIntent, IntentStatus, assert_return_shape_parses};

const RETURN_SHAPE_PROMPT: &str = "Emit a concrete Return shape expression for the packet output. A missing or unparsable shape is an obligation.";
const LIFT_TARGET: &str = "model:forge-lift";

/// Options for one FORGE prose-to-packet lift.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiftOptions {
    /// Stable symbolic name assigned to the compiled intent artifact.
    pub name: Symbol,
    /// Maximum repair attempts after the first candidate. Values above 2 are clamped.
    pub max_repairs: u8,
}

impl LiftOptions {
    /// Builds lift options with the default bounded repair count.
    pub fn new(name: Symbol) -> Self {
        Self {
            name,
            max_repairs: 1,
        }
    }
}

impl Default for LiftOptions {
    fn default() -> Self {
        Self::new(Symbol::qualified("forge", "lifted"))
    }
}

/// Compiles prose into a structurally checked candidate BRIDGE packet artifact.
///
/// The lift itself is a BRIDGE ASK exchange: the prose is supplied as fenced
/// data, the outer ASK return shape is `bridge/Packet`, and the returned packet
/// is accepted only after local return-shape parsing and `rx_check`.
pub fn forge_lift_once(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    prose: &str,
    opts: &LiftOptions,
) -> Result<CompiledIntent> {
    let (normalized, source) = normalize_prose(prose)?;
    let book = BridgeBook::standard();
    let policy = RepairPolicy::new(opts.max_repairs);
    let mut repair_report = None;
    let mut final_report = None;

    for attempt in 0..=policy.max_retries {
        let request = lift_request_packet(&normalized, &source, repair_report.as_ref(), attempt)?;
        let (reply, _) = run_bridge(cx, target, &book, request)?;
        let candidate = stamp_packet_cid(&candidate_packet_from_reply(&reply)?)?;
        let report = validate_candidate(cx, &book, &candidate)?;
        if report.accepted() {
            return compiled_intent(opts, source, candidate);
        }
        if attempt < policy.max_retries {
            repair_report = Some(report);
        } else {
            final_report = Some(report);
        }
    }

    let report = final_report.expect("bounded lift loop records the terminal report");
    Err(Error::Eval(format!(
        "forge lift failed after {} attempt(s): {}",
        usize::from(policy.max_retries) + 1,
        report_summary(&report)
    )))
}

fn lift_request_packet(
    normalized: &str,
    source: &ContentId,
    repair_report: Option<&BridgeReport>,
    attempt: u8,
) -> Result<BridgePacket> {
    let source_expr = Expr::String(normalized.to_owned());
    let (_, fenced_source) = fenced_expr("source", &source_expr)?;
    let given = BridgePart {
        id: Symbol::new("G1"),
        kind: Symbol::qualified("bridge", "Given"),
        payload: Expr::Map(vec![
            entry(
                "kind",
                Expr::Symbol(Symbol::qualified("forge", "ProseSource")),
            ),
            entry("content-id", Expr::String(content_id_string(source))),
            entry("prose", Expr::String(fenced_source)),
            entry(
                "return-shape-instruction",
                Expr::String(RETURN_SHAPE_PROMPT.to_owned()),
            ),
        ]),
    };

    let mut call = BridgeCallPayload::new(Symbol::qualified("forge", "lift"))
        .with_arg(fenced_arg("source", &source_expr)?)
        .with_arg(fenced_arg(
            "return-shape-instruction",
            &Expr::String(RETURN_SHAPE_PROMPT.to_owned()),
        )?);
    if let Some(report) = repair_report {
        call = call.with_arg(fenced_arg(&format!("repair-{attempt}"), &report.to_expr())?);
    }

    Ok(BridgePacket {
        header: BridgeHeader {
            cid: None,
            move_kind: Symbol::new("request"),
            from: "sim".to_owned(),
            to: vec![LIFT_TARGET.to_owned()],
            role: Symbol::new("implementer"),
            parents: Vec::new(),
            task: Symbol::new("C1"),
            output: Symbol::new("O1"),
            ceiling: vec![Symbol::qualified("ai", "run")],
            context: vec![Symbol::new("G1")],
            provenance: BridgeProvenance::default(),
        },
        body: vec![
            given,
            BridgePart {
                id: Symbol::new("C1"),
                kind: Symbol::qualified("bridge", "Call"),
                payload: call.to_expr(),
            },
            BridgePart {
                id: Symbol::new("O1"),
                kind: Symbol::qualified("bridge", "Return"),
                payload: Expr::Map(vec![
                    entry("codec", Expr::Symbol(Symbol::qualified("codec", "bridge"))),
                    entry("shape", Expr::Symbol(Symbol::qualified("bridge", "Packet"))),
                ]),
            },
        ],
        warrant: None,
    })
}

fn candidate_packet_from_reply(reply: &BridgePacket) -> Result<BridgePacket> {
    let output = reply
        .body
        .iter()
        .find(|part| part.id == reply.header.output)
        .ok_or_else(|| {
            Error::Eval(format!(
                "forge lift reply output part {} is missing",
                reply.header.output
            ))
        })?;
    if output.kind != Symbol::qualified("bridge", "Return") {
        return Err(Error::Eval(format!(
            "forge lift reply output part {} is {}, expected bridge/Return",
            output.id, output.kind
        )));
    }
    expr_to_packet(&output.payload)
}

pub(crate) fn validate_candidate(
    cx: &mut Cx,
    book: &BridgeBook,
    candidate: &BridgePacket,
) -> Result<BridgeReport> {
    let mut report = rx_check(cx, book, candidate, None)?;
    if let Err(err) = assert_return_shape_parses(candidate) {
        report.obligate(BridgeObligation::repair_packet(
            format!("body/{}/payload/shape", candidate.header.output),
            "return Shape does not parse",
            "parseable Shape expression",
            err.to_string(),
        ));
    }
    Ok(report)
}

pub(crate) fn compiled_intent(
    opts: &LiftOptions,
    source: ContentId,
    candidate: BridgePacket,
) -> Result<CompiledIntent> {
    Ok(CompiledIntent {
        name: opts.name.clone(),
        version: 1,
        source,
        packet: packet_content_id(&candidate)?,
        verifiers: candidate_verifiers(&candidate),
        probes: Vec::new(),
        status: IntentStatus::Candidate,
        compiler_card: None,
        approval: None,
    })
}

fn candidate_verifiers(packet: &BridgePacket) -> Vec<Symbol> {
    packet
        .body
        .iter()
        .filter(|part| {
            part.kind == Symbol::qualified("bridge", "Check")
                || part.kind == Symbol::qualified("bridge", "Evidence")
                || part.kind == Symbol::qualified("bridge", "Vote")
        })
        .map(|part| part.id.clone())
        .collect()
}

fn fenced_arg(name: &str, expr: &Expr) -> Result<BridgeCallArgument> {
    let (content_id, fenced) = fenced_expr(name, expr)?;
    Ok(BridgeCallArgument::new(
        Symbol::new(name),
        Symbol::qualified("codec", "json"),
        CallArgumentMedia::Text,
        content_id_string(&content_id),
        fenced,
    ))
}

fn fenced_expr(label: &str, expr: &Expr) -> Result<(ContentId, String)> {
    let content_id = content_id_for_expr(expr)?;
    let body = sim_codec_json::expr_to_json(expr).to_string();
    let fence = InjectionFence::for_content(&content_id);
    Ok((content_id, fence.wrap(label, &body)))
}

pub(crate) fn content_id_for_expr(expr: &Expr) -> Result<ContentId> {
    Datum::try_from(expr.clone())?.content_id()
}

pub(crate) fn report_summary(report: &BridgeReport) -> String {
    if report.obligations.is_empty() {
        return "no obligations".to_owned();
    }
    report
        .obligations
        .iter()
        .map(|obligation| {
            format!(
                "{}: {} (expected {}, actual {})",
                obligation.path, obligation.reason, obligation.expected, obligation.actual
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}
