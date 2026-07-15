use sim_codec_bridge::{
    BridgeBook, BridgeCallPayload, BridgePacket, OwnedSpan, assert_roundtrip,
    assert_total_ownership, encode_bridge_text, stamp_packet_cid, warrant_for_packet,
};
use sim_kernel::{Consistency, Cx, Error, EvalFabric, EvalMode, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{FENCE_DATA_RULE, ModelRequest};
use sim_value::build::entry;

use crate::brief::render_brief_sentences;
use crate::model::output_contract_for_packet;
use crate::rx::{bridge_rx_response, effective_caps, rx_check};

/// Canonicalizes, stamps, and locally validates a packet before transmission.
pub fn prepare_packet(
    cx: &mut Cx,
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<BridgePacket> {
    let mut packet = packet.canonicalized();
    packet.warrant = Some(warrant_for_packet(book, &packet)?);
    let packet = stamp_packet_cid(&packet)?;
    let report = rx_check(cx, book, &packet, None)?;
    if !report.accepted() {
        return Err(Error::Eval(format!(
            "bridge tx self-check failed: {:?}",
            report.obligations
        )));
    }
    assert_roundtrip(&packet, book)?;
    let (face, spans) = render_model_face(book, &packet)?;
    assert_total_ownership(&face, &spans)?;
    Ok(packet)
}

/// Renders the model-facing BRIDGE line face and ownership spans.
pub fn render_model_face(
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<(String, Vec<OwnedSpan>)> {
    let script = encode_bridge_text(packet, book)?;
    let mut face = script.clone();
    let mut spans = vec![OwnedSpan::Structural(script)];
    let sentences = render_brief_sentences(book, packet)?;
    if !sentences.is_empty() {
        let marker = "\nFLUENT\n".to_owned();
        face.push_str(&marker);
        spans.push(OwnedSpan::Structural(marker));
        for (id, sentence) in sentences {
            let text = format!("{sentence}\n");
            face.push_str(&text);
            spans.push(OwnedSpan::Frame { id, text });
        }
    }
    let fences = render_call_fences(packet)?;
    if !fences.is_empty() {
        let marker = "\nCALL-DATA\n".to_owned();
        face.push_str(&marker);
        spans.push(OwnedSpan::Structural(marker));
        for (id, text) in fences {
            face.push_str(&text);
            spans.push(OwnedSpan::Fence { id, text });
        }
    }
    Ok((face, spans))
}

/// Builds an eval request for a packet after running the TX self-check gate.
pub fn bridge_tx(cx: &mut Cx, book: &BridgeBook, packet: &BridgePacket) -> Result<EvalRequest> {
    let packet = prepare_packet(cx, book, packet)?;
    eval_request_for_checked_packet(cx, book, &packet)
}

/// Runs one checked BRIDGE exchange over a target eval fabric.
pub fn run_bridge(
    cx: &mut Cx,
    target: &dyn EvalFabric,
    book: &BridgeBook,
    packet: BridgePacket,
) -> Result<(BridgePacket, crate::BridgeReport)> {
    let packet = prepare_packet(cx, book, &packet)?;
    let caps = effective_caps(cx, &packet)?;
    cx.with_capabilities(caps, |cx| {
        let request = eval_request_for_checked_packet(cx, book, &packet)?;
        let reply = target.realize(cx, request)?;
        let response =
            sim_lib_agent_runner_core::ModelResponse::try_from(reply.value.object().as_expr(cx)?)?;
        bridge_rx_response(cx, book, &response, Some(&packet))
    })
}

pub(crate) fn eval_request_for_checked_packet(
    cx: &mut Cx,
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<EvalRequest> {
    let (face, _) = render_model_face(book, packet)?;
    let mut model = ModelRequest::new(
        Expr::String(face),
        vec![Expr::String(FENCE_DATA_RULE.to_owned())],
    );
    if let Some(cid) = &packet.header.cid {
        model.extra.push((
            Expr::Symbol(Symbol::new("bridge-cid")),
            Expr::String(cid.clone()),
        ));
    }
    output_contract_for_packet(packet)?.into_extra_entries(&mut model.extra);
    append_call_model_params(packet, &mut model.extra)?;
    let required_capabilities = effective_caps(cx, packet)?.iter().cloned().collect();
    Ok(EvalRequest {
        expr: Expr::from(model),
        result_shape: None,
        required_capabilities,
        deadline: None,
        consistency: Consistency::default(),
        mode: EvalMode::default(),
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        trace: false,
    })
}

fn render_call_fences(packet: &BridgePacket) -> Result<Vec<(String, String)>> {
    let mut fences = Vec::new();
    for part in &packet.body {
        if part.kind != Symbol::qualified("bridge", "Call") {
            continue;
        }
        let payload = BridgeCallPayload::from_expr(&part.payload)?;
        for arg in payload.args {
            let id = format!(
                "{}.{}",
                part.id.as_qualified_str(),
                arg.name.as_qualified_str()
            );
            fences.push((id.clone(), format!("[{id}]\n{}\n", arg.fenced)));
        }
    }
    Ok(fences)
}

fn append_call_model_params(packet: &BridgePacket, extra: &mut Vec<(Expr, Expr)>) -> Result<()> {
    let mut calls = Vec::new();
    for part in &packet.body {
        if part.kind != Symbol::qualified("bridge", "Call") {
            continue;
        }
        let payload = BridgeCallPayload::from_expr(&part.payload)?;
        calls.push(Expr::Map(vec![
            entry("part", Expr::Symbol(part.id.clone())),
            entry("name", Expr::Symbol(payload.name)),
            entry(
                "model-params",
                Expr::Map(
                    payload
                        .model_params
                        .into_iter()
                        .map(|(name, value)| (Expr::Symbol(name), value))
                        .collect(),
                ),
            ),
        ]));
    }
    if !calls.is_empty() {
        extra.push((
            Expr::Symbol(Symbol::new("bridge-calls")),
            Expr::Vector(calls),
        ));
    }
    Ok(())
}
