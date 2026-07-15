use sim_codec_bridge::BridgePacket;
use sim_kernel::{Error, Expr, Result, Symbol};
use sim_shape::parse_shape_expr;
use sim_value::access::field;

/// Verifies that a compiled packet declares a parseable return Shape.
pub fn assert_return_shape_parses(packet: &BridgePacket) -> Result<()> {
    let shape = return_shape_expr(packet)?;
    parse_shape_expr(shape)
        .map(|_| ())
        .map_err(|err| Error::Eval(format!("forge return Shape obligation: {err}")))
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
