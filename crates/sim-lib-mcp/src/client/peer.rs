use std::{collections::VecDeque, sync::Mutex};

use sim_codec_mcp::{McpEnvelope, McpErrorEnvelope, McpRequest, McpResponse};
use sim_kernel::{Cx, Error, Expr, Result};

use crate::McpRouter;

/// A peer an [`McpClient`](crate::McpClient) exchanges envelopes with.
pub trait McpClientPeer: Send + Sync {
    /// Sends `envelope` to the peer and returns its reply envelope.
    fn exchange(&self, cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope>;
}

pub(crate) fn request_peer(
    cx: &mut Cx,
    peer: &dyn McpClientPeer,
    id: Expr,
    method: &str,
    params: Expr,
) -> Result<Expr> {
    match peer.exchange(
        cx,
        McpEnvelope::Request(McpRequest {
            id,
            method: method.to_owned(),
            params,
        }),
    )? {
        McpEnvelope::Response(McpResponse { result, .. }) => Ok(result),
        McpEnvelope::Error(error) => Err(mcp_error(error)),
        _ => Err(Error::Eval(
            "foreign MCP peer returned non-response".to_owned(),
        )),
    }
}

/// In-process [`McpClientPeer`] backed by a local [`McpRouter`].
pub struct RouterMcpPeer {
    router: Mutex<McpRouter>,
}

impl RouterMcpPeer {
    /// Wraps `router` as a peer.
    pub fn new(router: McpRouter) -> Self {
        Self {
            router: Mutex::new(router),
        }
    }
}

impl McpClientPeer for RouterMcpPeer {
    fn exchange(&self, cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope> {
        self.router
            .lock()
            .map_err(|_| Error::PoisonedLock("mcp client router peer"))?
            .handle(cx, envelope)?
            .ok_or_else(|| Error::Eval("foreign MCP peer returned no response".to_owned()))
    }
}

/// [`McpClientPeer`] that replays a fixed sequence of recorded responses.
pub struct McpClientCassettePeer {
    frames: Mutex<VecDeque<McpCassetteFrame>>,
}

struct McpCassetteFrame {
    method: String,
    result: Expr,
}

impl McpClientCassettePeer {
    /// Creates a cassette peer that replies with `frames` in order, each a
    /// `(method, result)` pair matched against the incoming request method.
    pub fn new(frames: Vec<(String, Expr)>) -> Self {
        Self {
            frames: Mutex::new(
                frames
                    .into_iter()
                    .map(|(method, result)| McpCassetteFrame { method, result })
                    .collect(),
            ),
        }
    }

    /// Returns the number of unconsumed recorded frames.
    pub fn remaining(&self) -> Result<usize> {
        Ok(self
            .frames
            .lock()
            .map_err(|_| Error::PoisonedLock("mcp client cassette"))?
            .len())
    }
}

impl McpClientPeer for McpClientCassettePeer {
    fn exchange(&self, _cx: &mut Cx, envelope: McpEnvelope) -> Result<McpEnvelope> {
        let McpEnvelope::Request(request) = envelope else {
            return Err(Error::TypeMismatch {
                expected: "MCP request",
                found: "non-request",
            });
        };
        let frame = self
            .frames
            .lock()
            .map_err(|_| Error::PoisonedLock("mcp client cassette"))?
            .pop_front()
            .ok_or_else(|| Error::Eval("MCP client cassette exhausted".to_owned()))?;
        if frame.method != request.method {
            return Err(Error::Eval(format!(
                "MCP client cassette expected {}, got {}",
                frame.method, request.method
            )));
        }
        Ok(McpEnvelope::Response(McpResponse {
            id: request.id,
            result: frame.result,
        }))
    }
}

fn mcp_error(error: McpErrorEnvelope) -> Error {
    Error::Eval(format!(
        "foreign MCP error {}: {}",
        error.error.code, error.error.message
    ))
}
