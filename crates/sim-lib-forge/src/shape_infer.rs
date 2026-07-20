use sim_codec_bridge::BridgePacket;
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_shape::{parse_shape_expr, shape_grammar_graph};
use sim_value::access::field;

/// Verifies that a compiled packet declares a parseable return Shape.
pub fn assert_return_shape_parses(packet: &BridgePacket) -> Result<()> {
    let shape = return_shape_expr(packet)?;
    let parsed = parse_shape_expr(shape)
        .map_err(|err| Error::Eval(format!("forge return Shape obligation: {err}")))?;
    if return_grammar_requested(packet)? {
        shape_grammar_graph(parsed.as_ref())
            .map_err(|err| Error::Eval(format!("forge return Shape grammar obligation: {err}")))?;
    }
    Ok(())
}

pub(crate) fn return_shape_expr(packet: &BridgePacket) -> Result<&Expr> {
    let output = packet
        .body
        .iter()
        .find(|part| part.id == packet.header.output)
        .ok_or_else(|| {
            Error::Eval(format!(
                "forge return Shape obligation: output part {} is missing",
                packet.header.output
            ))
        })?;
    if output.kind != Symbol::qualified("bridge", "Return") {
        return Err(Error::Eval(format!(
            "forge return Shape obligation: output part {} is {}, expected bridge/Return",
            output.id, output.kind
        )));
    }
    field(&output.payload, "shape").ok_or_else(|| {
        Error::Eval(format!(
            "forge return Shape obligation: output part {} has no shape",
            output.id
        ))
    })
}

fn return_grammar_requested(packet: &BridgePacket) -> Result<bool> {
    let output = packet
        .body
        .iter()
        .find(|part| part.id == packet.header.output)
        .ok_or_else(|| {
            Error::Eval(format!(
                "forge return Shape obligation: output part {} is missing",
                packet.header.output
            ))
        })?;
    Ok(
        matches!(field(&output.payload, "grammar"), Some(Expr::Bool(true)))
            || matches!(
                field(&output.payload, "output-grammar-required"),
                Some(Expr::Bool(true))
            ),
    )
}
