use crate::CapabilityPack;
use sim_kernel::Expr;
/// Pack codec refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Wrong schema version.
    UnsupportedVersion(u64),
    /// Invalid Citizen field representation.
    Invalid(String),
}
/// Encodes a pack as its stable Citizen field expression for general-purpose Lisp codecs.
pub fn encode_pack(pack: &CapabilityPack) -> Expr {
    sim_citizen::CitizenField::encode_field(pack)
}
/// Decodes the stable versioned pack field expression.
pub fn decode_pack(version: u64, expr: &Expr) -> Result<CapabilityPack, CodecError> {
    if version != crate::CURRENT_PACK_VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    sim_citizen::CitizenField::decode_field_expr(expr, "capability-pack")
        .map_err(|e| CodecError::Invalid(e.to_string()))
}
