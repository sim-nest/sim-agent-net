use sim_codec_bridge::{
    BridgeBook, BridgePacket, OwnedSpan, assert_roundtrip, assert_total_ownership,
    encode_bridge_text, stamp_packet_cid,
};
use sim_kernel::{Consistency, Cx, Error, EvalFabric, EvalMode, EvalRequest, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{FENCE_DATA_RULE, ModelRequest};

use crate::model::output_contract_for_packet;
use crate::rx::{bridge_rx_response, effective_caps, rx_check};

/// Canonicalizes, stamps, and locally validates a packet before transmission.
pub fn prepare_packet(
    cx: &mut Cx,
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<BridgePacket> {
    let packet = stamp_packet_cid(&packet.canonicalized())?;
    assert_roundtrip(&packet, book)?;
    let (face, spans) = render_model_face(book, &packet)?;
    assert_total_ownership(&face, &spans)?;
    let report = rx_check(cx, book, &packet, None)?;
    if !report.accepted() {
        return Err(Error::Eval(format!(
            "bridge tx self-check failed: {:?}",
            report.obligations
        )));
    }
    Ok(packet)
}

/// Renders the model-facing BRIDGE line face and ownership spans.
pub fn render_model_face(
    book: &BridgeBook,
    packet: &BridgePacket,
) -> Result<(String, Vec<OwnedSpan>)> {
    let face = encode_bridge_text(packet, book)?;
    Ok((face.clone(), vec![OwnedSpan::Structural(face)]))
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
