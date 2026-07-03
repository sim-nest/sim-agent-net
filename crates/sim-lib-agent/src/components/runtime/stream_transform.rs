use sim_kernel::{Cx, Error, Expr, Result};
use sim_lib_server::{FrameKind, ServerFrame, encode_frame_payload, stream_frame_to_expr};
use sim_lib_stream_core::StreamPacket;

pub(super) fn transform_stream_expr(
    cx: &mut Cx,
    frame: ServerFrame,
    transform: impl FnOnce(&mut Cx, Expr) -> Result<Expr>,
) -> Result<ServerFrame> {
    if frame.kind == FrameKind::Response {
        return Err(Error::Eval(
            "stream transforms do not accept response fallback frames".to_owned(),
        ));
    }
    let Some(expr) = stream_frame_to_expr(cx, &frame)? else {
        return Ok(frame);
    };
    let transformed = transform(cx, expr)?;
    replace_payload(cx, frame, transformed)
}

pub(super) fn transform_data_payloads(
    cx: &mut Cx,
    frame: ServerFrame,
    transform: impl FnOnce(&mut Cx, Expr) -> Result<Expr>,
) -> Result<ServerFrame> {
    transform_stream_expr(cx, frame, |cx, expr| {
        match StreamPacket::try_from(expr.clone()) {
            Ok(StreamPacket::Data(packet)) => {
                let payload = transform(cx, packet.payload)?;
                Ok(StreamPacket::data(packet.kind, payload).to_expr())
            }
            Ok(packet) => Ok(packet.to_expr()),
            Err(_) => transform(cx, expr),
        }
    })
}

pub(super) fn replace_payload(
    cx: &mut Cx,
    mut frame: ServerFrame,
    expr: Expr,
) -> Result<ServerFrame> {
    frame.payload = encode_frame_payload(
        cx,
        &frame.codec,
        &expr,
        sim_kernel::EncodeOptions::default(),
    )?;
    Ok(frame)
}
