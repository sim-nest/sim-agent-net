//! Shared reply-frame helper for agent runtime components.
//!
//! Almost every agent component that answers a server frame ends with the same
//! block: drain pending diagnostics, wrap the value in an [`EvalReply`] with no
//! trace, and encode it on the request frame's codec. This is the one home for
//! that block.

use sim_kernel::{Consistency, Cx, EvalReply, Result, Value};
use sim_lib_server::{ServerFrame, server_frame_from_reply};

/// Drain pending diagnostics and encode `value` as a reply frame on `frame`'s
/// codec, preserving the request's consistency. Equivalent to the hand-written
/// `take_diagnostics` + `EvalReply { value, diagnostics, trace: None }` +
/// `server_frame_from_reply` block it replaces.
pub(crate) fn reply_frame(
    cx: &mut Cx,
    frame: &ServerFrame,
    value: Value,
    consistency: Consistency,
) -> Result<ServerFrame> {
    let diagnostics = cx.take_diagnostics();
    server_frame_from_reply(
        cx,
        &frame.codec,
        EvalReply {
            value,
            diagnostics,
            trace: None,
        },
        consistency,
    )
}
