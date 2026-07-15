use sim_codec_bridge::{BridgePacket, BridgePart};
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_lib_agent_runner_core::{ModelResponse, OutputContract};
use sim_lib_stream_fabric::ContentKey;
use sim_value::access::field;

use crate::rx::{shape_from_contract_expr, terminal_bridge_text as rx_terminal_bridge_text};
use crate::tx::eval_request_for_checked_packet;

/// Returns the terminal BRIDGE text from a model response.
pub fn terminal_bridge_text(response: &ModelResponse) -> Result<&str> {
    rx_terminal_bridge_text(response)
}

/// Decodes the terminal content item of a model response as a BRIDGE packet.
pub fn terminal_response_packet(
    response: &ModelResponse,
    book: &sim_codec_bridge::BridgeBook,
) -> Result<BridgePacket> {
    sim_codec_bridge::decode_bridge_text(terminal_bridge_text(response)?, book)
}

/// Derives the content key for a checked BRIDGE model request.
pub fn bridge_request_content_key(
    cx: &mut sim_kernel::Cx,
    book: &sim_codec_bridge::BridgeBook,
    packet: &BridgePacket,
) -> Result<ContentKey> {
    let request = eval_request_for_checked_packet(cx, book, packet)?;
    Ok(ContentKey::from_request(&request))
}

/// Builds the model output contract declared by a packet's output part.
pub fn output_contract_for_packet(packet: &BridgePacket) -> Result<OutputContract> {
    let output = output_part(packet).ok_or_else(|| {
        Error::Eval(format!(
            "BRIDGE packet output part {} is missing",
            packet.header.output
        ))
    })?;
    let codec = match field(&output.payload, "codec") {
        Some(Expr::Symbol(symbol)) => symbol.clone(),
        None => Symbol::qualified("codec", "bridge"),
        Some(other) => {
            return Err(Error::Eval(format!(
                "BRIDGE Return codec must be a symbol, found {other:?}"
            )));
        }
    };
    let shape_expr = field(&output.payload, "shape")
        .cloned()
        .unwrap_or_else(|| Expr::Symbol(Symbol::qualified("core", "Any")));
    let required = !matches!(field(&output.payload, "strict"), Some(Expr::Bool(false)));
    let Some(shape) = shape_from_contract_expr(&shape_expr) else {
        return Ok(OutputContract::new(codec, shape_expr, None, required));
    };
    Ok(OutputContract::for_shape(
        codec,
        shape_expr,
        shape.as_ref(),
        required,
    ))
}

fn output_part(packet: &BridgePacket) -> Option<&BridgePart> {
    packet
        .body
        .iter()
        .find(|part| part.id == packet.header.output)
}
