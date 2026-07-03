use sim_codec_binary::encode_frame;
use sim_kernel::{ContentId, Datum, Expr, Result};

use crate::objects::GatewayRequest;

/// Field names stripped from a request before computing its content id.
///
/// These carry per-receipt metadata (ids and timestamps) that must not affect
/// whether two requests are considered content-identical.
pub const VOLATILE_REQUEST_FIELDS: &[&str] = &[
    "id",
    "timestamp",
    "timestamp-ms",
    "created-at-ms",
    "updated-at-ms",
];

/// Computes the content id of a [`GatewayRequest`], ignoring volatile fields.
pub fn request_content_id(request: &GatewayRequest) -> Result<ContentId> {
    request_expr_content_id(&request.to_expr())
}

/// Computes the content id of a request expression after normalizing away its
/// volatile fields.
pub fn request_expr_content_id(expr: &Expr) -> Result<ContentId> {
    content_id_for_expr(&normalize_request_expr(expr))
}

/// Computes the content id of an arbitrary expression by encoding it to a
/// binary frame and hashing the resulting bytes.
pub fn content_id_for_expr(expr: &Expr) -> Result<ContentId> {
    let frame = encode_frame(expr)?;
    Datum::Bytes(frame.0).content_id()
}

/// Returns a copy of a request map with [`VOLATILE_REQUEST_FIELDS`] removed.
///
/// Non-map expressions are returned unchanged.
pub fn normalize_request_expr(expr: &Expr) -> Expr {
    let Expr::Map(entries) = expr else {
        return expr.clone();
    };
    Expr::Map(
        entries
            .iter()
            .filter(|(key, _)| !is_volatile_request_key(key))
            .cloned()
            .collect(),
    )
}

fn is_volatile_request_key(key: &Expr) -> bool {
    let name = match key {
        Expr::String(name) => name.as_str(),
        Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
        _ => return false,
    };
    VOLATILE_REQUEST_FIELDS.contains(&name)
}
