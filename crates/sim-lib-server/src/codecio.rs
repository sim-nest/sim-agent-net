use sim_codec::{DecodeLimits, Input, Output, decode_with_codec_and_limits, encode_with_codec};
use sim_kernel::{Cx, EncodeOptions, Expr, ReadPolicy, Result, Symbol};

/// Encodes an expression with `codec` and returns the frame payload bytes.
///
/// Text output is returned as its UTF-8 bytes; byte output is returned as-is.
pub fn encode_frame_payload(
    cx: &mut Cx,
    codec: &Symbol,
    expr: &Expr,
    options: EncodeOptions,
) -> Result<Vec<u8>> {
    match encode_with_codec(cx, codec, expr, options)? {
        Output::Text(text) => Ok(text.into_bytes()),
        Output::Bytes(bytes) => Ok(bytes),
    }
}

/// Decodes a frame payload back into an expression using `codec`.
///
/// The `read_policy` and `limits` bound how the payload bytes are parsed.
pub fn decode_frame_payload(
    cx: &mut Cx,
    codec: &Symbol,
    payload: &[u8],
    read_policy: ReadPolicy,
    limits: DecodeLimits,
) -> Result<Expr> {
    decode_with_codec_and_limits(
        cx,
        codec,
        Input::Bytes(payload.to_vec()),
        read_policy,
        limits,
    )
}
