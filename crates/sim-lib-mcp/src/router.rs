#[cfg(feature = "stream")]
use sim_codec_mcp::CANCELLED;
use sim_codec_mcp::{
    CAPABILITY_DENIED, EXECUTION_ERROR, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST,
    METHOD_NOT_FOUND, McpEnvelope, McpError, McpErrorEnvelope, McpNotification, McpRequest,
    McpResponse, RATE_LIMITED, envelope_to_expr, expr_to_envelope,
};
use sim_kernel::{Cx, Error, Expr, Result};

use crate::McpSession;
use crate::methods::{core, prompts, resources, tools};
use crate::session::McpBoundaryLimit;

/// Optional single reply envelope produced by routing one request.
pub type RouterReply = Option<McpEnvelope>;
/// Ordered reply envelopes (notifications then final response) for one request.
pub type RouterReplies = Vec<McpEnvelope>;

struct DispatchReply {
    result: Expr,
    notifications: RouterReplies,
}

/// Dispatches MCP request/notification envelopes against a session.
pub struct McpRouter {
    session: McpSession,
}

impl McpRouter {
    /// Creates a router bound to `session`.
    pub fn new(session: McpSession) -> Self {
        Self { session }
    }

    /// Creates a router over a permissive fixture session.
    pub fn fixture() -> Self {
        Self::new(McpSession::fixture())
    }

    /// Returns the underlying session.
    pub fn session(&self) -> &McpSession {
        &self.session
    }

    /// Returns a mutable reference to the underlying session.
    pub fn session_mut(&mut self) -> &mut McpSession {
        &mut self.session
    }

    /// Decodes one envelope [`Expr`], routes it, and encodes the final reply.
    pub fn handle_expr(&mut self, cx: &mut Cx, expr: Expr) -> Result<Option<Expr>> {
        let envelope = match expr_to_envelope(&expr) {
            Ok(envelope) => envelope,
            Err(error) => {
                return Ok(Some(envelope_to_expr(&invalid_request_error(
                    Expr::Nil,
                    error,
                ))));
            }
        };
        Ok(self
            .handle(cx, envelope)?
            .map(|reply| envelope_to_expr(&reply)))
    }

    /// Decodes one envelope [`Expr`], routes it, and encodes every reply
    /// (notifications and the final response) in order.
    pub fn handle_exprs(&mut self, cx: &mut Cx, expr: Expr) -> Result<Vec<Expr>> {
        let envelope = match expr_to_envelope(&expr) {
            Ok(envelope) => envelope,
            Err(error) => {
                return Ok(vec![envelope_to_expr(&invalid_request_error(
                    Expr::Nil,
                    error,
                ))]);
            }
        };
        Ok(self
            .handle_many(cx, envelope)?
            .into_iter()
            .map(|reply| envelope_to_expr(&reply))
            .collect())
    }

    /// Routes `envelope` and returns only the final response or error reply.
    pub fn handle(&mut self, cx: &mut Cx, envelope: McpEnvelope) -> Result<RouterReply> {
        Ok(self
            .handle_many(cx, envelope)?
            .into_iter()
            .rev()
            .find(is_final_reply))
    }

    /// Routes `envelope` and returns every reply it produces, in order.
    pub fn handle_many(&mut self, cx: &mut Cx, envelope: McpEnvelope) -> Result<RouterReplies> {
        match envelope {
            McpEnvelope::Request(request) => self.handle_request(cx, request),
            McpEnvelope::Notification(notification) => {
                self.handle_notification(cx, notification);
                Ok(Vec::new())
            }
            McpEnvelope::Response(_) | McpEnvelope::Error(_) => Ok(Vec::new()),
        }
    }

    fn handle_request(&mut self, cx: &mut Cx, request: McpRequest) -> Result<RouterReplies> {
        let id = request.id.clone();
        #[cfg(feature = "cassette")]
        let request_envelope = McpEnvelope::Request(request.clone());
        if let Err(limit) = self.session.admit_request(&id) {
            let reply = boundary_limit_error(id, limit);
            #[cfg(feature = "cassette")]
            self.audit_request(&request.method, "boundary", limit.audit_outcome());
            return Ok(vec![reply]);
        }
        #[cfg(feature = "cassette")]
        if let Some(replies) = self.replay_request(&request_envelope, &request.method)? {
            self.session.end_request(&id);
            return Ok(replies);
        }
        #[cfg(feature = "stream")]
        if self.session.request_cancelled(&id) {
            self.session.end_request(&id);
            return Ok(vec![cancelled_error(id, "request cancelled")]);
        }
        let result = self.dispatch_request(cx, &request.method, request.params);
        #[cfg(feature = "stream")]
        let cancelled = self.session.request_cancelled(&id);
        self.session.end_request(&id);
        #[cfg(feature = "stream")]
        if cancelled {
            return Ok(vec![cancelled_error(id, "request cancelled")]);
        }
        let replies = match result {
            Ok(reply) => {
                let mut replies = reply.notifications;
                replies.push(McpEnvelope::Response(McpResponse {
                    id,
                    result: reply.result,
                }));
                replies
            }
            Err(error) if matches!(error, Error::UnknownFunction { .. }) => {
                vec![method_not_found_error(id, error)]
            }
            Err(error) => vec![error_response(id, error)],
        };
        #[cfg(feature = "cassette")]
        self.record_request(&request_envelope, &request.method, &replies)?;
        Ok(replies)
    }

    fn handle_notification(&mut self, _cx: &mut Cx, notification: McpNotification) {
        match notification.method.as_str() {
            "initialized" | "notifications/initialized" => {
                core::initialized(&mut self.session);
            }
            "shutdown" => {
                core::shutdown(&mut self.session);
            }
            #[cfg(feature = "stream")]
            "notifications/cancelled" => {
                let _ = crate::stream::apply_cancel_notification(
                    &mut self.session,
                    notification.params,
                );
            }
            _ => {}
        }
    }

    fn dispatch_request(
        &mut self,
        cx: &mut Cx,
        method: &str,
        params: Expr,
    ) -> Result<DispatchReply> {
        let (result, notifications) = match method {
            "initialize" => (core::initialize(&mut self.session, params)?, Vec::new()),
            "initialized" | "notifications/initialized" => {
                (core::initialized(&mut self.session), Vec::new())
            }
            "ping" => (core::ping(), Vec::new()),
            "shutdown" => (core::shutdown(&mut self.session), Vec::new()),
            "resources/list" => (resources::list(cx, &self.session)?, Vec::new()),
            "resources/read" => (resources::read(cx, &self.session, params)?, Vec::new()),
            "prompts/list" => (prompts::list(cx, &self.session)?, Vec::new()),
            "prompts/get" => (prompts::get(cx, &self.session, params)?, Vec::new()),
            "tools/list" => (tools::list(cx, &self.session)?, Vec::new()),
            #[cfg(feature = "stream")]
            "tools/call" => {
                let progress_token = crate::stream::progress_token_from_params(&params);
                tools::call_with_stream(cx, &mut self.session, params, progress_token)?
            }
            #[cfg(not(feature = "stream"))]
            "tools/call" => (tools::call(cx, &self.session, params)?, Vec::new()),
            _ => Err(Error::UnknownFunction {
                function: sim_kernel::Symbol::new(method.to_owned()),
            })?,
        };
        Ok(DispatchReply {
            result,
            notifications,
        })
    }
}

fn is_final_reply(envelope: &McpEnvelope) -> bool {
    matches!(envelope, McpEnvelope::Response(_) | McpEnvelope::Error(_))
}

fn invalid_request_error(id: Expr, error: Error) -> McpEnvelope {
    error_envelope(id, INVALID_REQUEST, "invalid request", error.to_string())
}

fn method_not_found_error(id: Expr, error: Error) -> McpEnvelope {
    error_envelope(id, METHOD_NOT_FOUND, "method not found", error.to_string())
}

fn error_response(id: Expr, error: Error) -> McpEnvelope {
    if let Some(data) = crate::uri::not_found_error_data(&error) {
        return error_envelope_data(id, INVALID_PARAMS, "not found", data);
    }
    let (code, message) = error_code_and_message(&error);
    error_envelope(id, code, message, error.to_string())
}

#[cfg(feature = "stream")]
fn cancelled_error(id: Expr, detail: &str) -> McpEnvelope {
    error_envelope(id, CANCELLED, "cancelled", detail.to_owned())
}

fn boundary_limit_error(id: Expr, limit: McpBoundaryLimit) -> McpEnvelope {
    match limit {
        McpBoundaryLimit::Deadline => error_envelope(
            id,
            EXECUTION_ERROR,
            "deadline exceeded",
            "deadline exceeded".to_owned(),
        ),
        McpBoundaryLimit::Rate => error_envelope(
            id,
            RATE_LIMITED,
            "rate limited",
            "rate limit exceeded".to_owned(),
        ),
        McpBoundaryLimit::ActiveRequests => error_envelope(
            id,
            RATE_LIMITED,
            "rate limited",
            "active request limit exceeded".to_owned(),
        ),
    }
}

fn error_envelope(id: Expr, code: i64, message: &str, detail: String) -> McpEnvelope {
    error_envelope_data(id, code, message, Expr::String(detail))
}

#[cfg(feature = "cassette")]
impl McpRouter {
    fn replay_request(
        &mut self,
        request: &McpEnvelope,
        method: &str,
    ) -> Result<Option<RouterReplies>> {
        let Some(cassette) = self.session.cassette_mut() else {
            return Ok(None);
        };
        let Some(replies) = cassette.replay(request)? else {
            return Ok(None);
        };
        if let Some(operation) = auditable_operation(method) {
            cassette.record_audit(method, operation, "replay");
        }
        Ok(Some(replies))
    }

    fn record_request(
        &mut self,
        request: &McpEnvelope,
        method: &str,
        replies: &[McpEnvelope],
    ) -> Result<()> {
        let Some(cassette) = self.session.cassette_mut() else {
            return Ok(());
        };
        cassette.record_exchange(request, replies)?;
        if let Some(operation) = auditable_operation(method) {
            cassette.record_audit(method, operation, reply_outcome(replies));
        }
        Ok(())
    }

    fn audit_request(&mut self, method: &str, operation: &str, outcome: &str) {
        if let Some(cassette) = self.session.cassette_mut() {
            cassette.record_audit(method, operation, outcome);
        }
    }
}

#[cfg(feature = "cassette")]
fn auditable_operation(method: &str) -> Option<&'static str> {
    match method {
        "tools/call" => Some("tools/call"),
        "resources/read" => Some("resources/read"),
        "prompts/get" => Some("prompts/get"),
        "sampling/createMessage" => Some("sampling"),
        _ => None,
    }
}

#[cfg(feature = "cassette")]
fn reply_outcome(replies: &[McpEnvelope]) -> &'static str {
    if replies
        .iter()
        .any(|reply| matches!(reply, McpEnvelope::Error(_)))
    {
        "error"
    } else {
        "ok"
    }
}

impl McpBoundaryLimit {
    #[cfg(feature = "cassette")]
    fn audit_outcome(self) -> &'static str {
        match self {
            Self::Deadline => "deadline-denied",
            Self::Rate => "rate-denied",
            Self::ActiveRequests => "active-denied",
        }
    }
}

fn error_envelope_data(id: Expr, code: i64, message: &str, data: Expr) -> McpEnvelope {
    McpEnvelope::Error(McpErrorEnvelope {
        id,
        error: McpError {
            code,
            message: message.to_owned(),
            data,
        },
    })
}

fn error_code_and_message(error: &Error) -> (i64, &'static str) {
    match error {
        Error::CapabilityDenied { .. } | Error::TrustDenied { .. } => {
            (CAPABILITY_DENIED, "capability denied")
        }
        Error::TypeMismatch { .. }
        | Error::WrongShape { .. }
        | Error::NoMatchingOverload { .. } => (INVALID_PARAMS, "invalid params"),
        Error::UnknownFunction { .. } => (METHOD_NOT_FOUND, "method not found"),
        Error::HostError(_) | Error::PoisonedLock(_) => (INTERNAL_ERROR, "internal error"),
        _ => (EXECUTION_ERROR, "execution error"),
    }
}
