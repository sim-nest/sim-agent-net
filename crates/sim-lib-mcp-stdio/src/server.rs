use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    sync::{Arc, mpsc},
    thread,
};

use serde_json::{Value, json};
use sim_cancel::{Cancellation, CancellationReason};
use sim_codec_json::{JsonProjectionMode, project_expr_to_json, project_json_to_expr};
use sim_codec_mcp::{McpEnvelope, envelope_to_expr, expr_to_envelope};
use sim_kernel::Cx;
use sim_lib_mcp::{McpService, RequestContext};

use crate::{FrameError, JsonLineFramer};

// conformance: MCP stdio interleaves request contexts and cancels only the addressed live id.

/// Bounded, non-protocol diagnostic destination (normally stderr).
pub trait DiagnosticSink {
    /// Records one already bounded diagnostic line.
    fn diagnostic(&mut self, message: &str);
}

impl<T: Write> DiagnosticSink for T {
    fn diagnostic(&mut self, message: &str) {
        let _ = writeln!(self, "{message}");
    }
}

/// One complete modern dispatch, including untouched `_meta` and fresh cancellation.
#[derive(Clone, Debug)]
pub struct DispatchCall {
    /// Full decoded request object.
    pub message: Value,
    /// Full `_meta` value, or null when absent.
    pub meta: Value,
    /// Request-owned cancellation token.
    pub cancellation: Cancellation,
}

/// Application dispatch failure; adapters map it to JSON-RPC error output.
#[derive(Clone, Debug)]
pub struct DispatchError(pub String);

/// Stateless application boundary used by the stdio lifetime adapter.
pub trait Dispatch: Send + Sync + 'static {
    /// Executes one request. Implementations must create a fresh `Cx` and bind
    /// `call.cancellation` into that request context.
    fn dispatch(&self, call: DispatchCall) -> Result<Vec<Value>, DispatchError>;
}

type ContextFactory = dyn Fn(&Value, &Value) -> Result<RequestContext, DispatchError> + Send + Sync;

/// Concrete stateless service composition. Both factories are explicit so the
/// host decides how `_meta` authenticates a principal and how cancellation is
/// installed in each newly created `Cx`.
pub struct ModernDispatch {
    service: Arc<McpService>,
    cx_factory: Arc<dyn Fn(&Cancellation) -> Cx + Send + Sync>,
    context_factory: Arc<ContextFactory>,
}

/// Explicit initialize-era composition, available only with the `legacy`
/// feature. The mutex protects the compatibility adapter's deliberate
/// connection facts; it is never created by observing a modern request.
#[cfg(feature = "legacy")]
pub struct LegacyDispatch {
    connection: std::sync::Mutex<sim_lib_mcp_legacy::LegacyConnection>,
    cx_factory: Arc<dyn Fn(&Cancellation) -> Cx + Send + Sync>,
}

#[cfg(feature = "legacy")]
impl LegacyDispatch {
    /// Installs one construction-time legacy connection and fresh-context policy.
    pub fn new(
        connection: sim_lib_mcp_legacy::LegacyConnection,
        cx_factory: Arc<dyn Fn(&Cancellation) -> Cx + Send + Sync>,
    ) -> Self {
        Self {
            connection: std::sync::Mutex::new(connection),
            cx_factory,
        }
    }
}

#[cfg(feature = "legacy")]
impl Dispatch for LegacyDispatch {
    fn dispatch(&self, call: DispatchCall) -> Result<Vec<Value>, DispatchError> {
        let mut cx = (self.cx_factory)(&call.cancellation);
        let mut wire = call.message;
        if let Some(object) = wire.as_object_mut() {
            if let Some(version) = object.remove("jsonrpc") {
                object.insert("mcp".into(), version);
            }
        }
        let envelope = expr_to_envelope(&project_json_to_expr(
            &wire,
            JsonProjectionMode::UntaggedInterop,
        ))
        .map_err(|e| DispatchError(e.to_string()))?;
        let responses = self
            .connection
            .lock()
            .map_err(|_| DispatchError("legacy connection lock poisoned".into()))?
            .handle_envelope(&mut cx, envelope)
            .map_err(|e| DispatchError(e.to_string()))?;
        Ok(responses
            .into_iter()
            .map(|envelope| {
                let mut value = project_expr_to_json(
                    &envelope_to_expr(&envelope),
                    JsonProjectionMode::UntaggedInterop,
                );
                if let Some(object) = value.as_object_mut() {
                    if let Some(version) = object.remove("mcp") {
                        object.insert("jsonrpc".into(), version);
                    }
                }
                value
            })
            .collect())
    }
}

impl ModernDispatch {
    /// Constructs a modern dispatcher from immutable service and host policy.
    pub fn new(
        service: Arc<McpService>,
        cx_factory: Arc<dyn Fn(&Cancellation) -> Cx + Send + Sync>,
        context_factory: Arc<ContextFactory>,
    ) -> Self {
        Self {
            service,
            cx_factory,
            context_factory,
        }
    }
}

impl Dispatch for ModernDispatch {
    fn dispatch(&self, call: DispatchCall) -> Result<Vec<Value>, DispatchError> {
        let context = (self.context_factory)(&call.message, &call.meta)?;
        let mut cx = (self.cx_factory)(&call.cancellation);
        let mut wire = call.message;
        if let Some(object) = wire.as_object_mut()
            && let Some(version) = object.remove("jsonrpc")
        {
            object.insert("mcp".into(), version);
        }
        let expr = project_json_to_expr(&wire, JsonProjectionMode::UntaggedInterop);
        let McpEnvelope::Request(request) =
            expr_to_envelope(&expr).map_err(|e| DispatchError(e.to_string()))?
        else {
            return Err(DispatchError(
                "modern dispatch requires a request envelope".into(),
            ));
        };
        self.service
            .handle(&mut cx, &context, request)
            .map_err(|e| DispatchError(e.to_string()))?
            .map(|envelope| {
                let mut value = project_expr_to_json(
                    &envelope_to_expr(&envelope),
                    JsonProjectionMode::UntaggedInterop,
                );
                if let Some(object) = value.as_object_mut()
                    && let Some(version) = object.remove("mcp")
                {
                    object.insert("jsonrpc".into(), version);
                }
                Ok(value)
            })
            .collect()
    }
}

/// Construction-time legacy policy. It is never inferred from traffic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LegacyMode {
    /// Reject lifecycle negotiation and remain modern.
    #[default]
    Disabled,
    /// Route initialize-era lifecycle through an explicitly installed adapter.
    Enabled,
}

/// Fixed resource bounds and compatibility policy.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// Maximum bytes in one input or output line.
    pub max_frame_bytes: usize,
    /// Maximum pending serialized output messages.
    pub write_queue_depth: usize,
    /// Maximum diagnostic bytes per event.
    pub max_diagnostic_bytes: usize,
    /// Construction-time compatibility choice.
    pub legacy: LegacyMode,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024,
            write_queue_depth: 64,
            max_diagnostic_bytes: 1024,
            legacy: LegacyMode::Disabled,
        }
    }
}

/// Terminal counts from one server lifetime.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServerSummary {
    /// Input messages admitted.
    pub messages_read: usize,
    /// Protocol messages written.
    pub messages_written: usize,
    /// Unknown or late cancellations observed.
    pub unknown_cancellations: usize,
}

/// Server-side stdio lifetime adapter.
pub struct StdioServer<D> {
    dispatch: Arc<D>,
    options: ServerOptions,
}

impl<D: Dispatch> StdioServer<D> {
    /// Builds the adapter with immutable process-lifetime policy.
    pub fn new(dispatch: Arc<D>, options: ServerOptions) -> Result<Self, FrameError> {
        JsonLineFramer::new(options.max_frame_bytes)?;
        if options.write_queue_depth == 0 {
            return Err(FrameError::InvalidJson(
                "write queue depth must be non-zero".into(),
            ));
        }
        Ok(Self { dispatch, options })
    }

    /// Serves until clean EOF or a terminal framing/write error.
    pub fn serve<R: BufRead, W: Write + Send + 'static>(
        &self,
        mut input: R,
        output: W,
        diagnostics: &mut dyn DiagnosticSink,
    ) -> Result<ServerSummary, FrameError> {
        let framer = JsonLineFramer::new(self.options.max_frame_bytes)?;
        let (write_tx, write_rx) = mpsc::sync_channel::<Value>(self.options.write_queue_depth);
        let writer_framer = framer;
        let writer = thread::spawn(move || {
            let mut output = output;
            let mut count = 0usize;
            for value in write_rx {
                writer_framer.write(&mut output, &value)?;
                count += 1;
            }
            output.flush().map_err(FrameError::Io)?;
            Ok::<_, FrameError>(count)
        });
        let (done_tx, done_rx) =
            mpsc::channel::<(String, Value, Result<Vec<Value>, DispatchError>)>();
        let mut active = BTreeMap::<String, Cancellation>::new();
        let mut summary = ServerSummary::default();
        let mut workers = Vec::new();
        let mut terminal_error = None;
        'input: loop {
            while let Ok((key, id, result)) = done_rx.try_recv() {
                active.remove(&key);
                if enqueue_result(&write_tx, &id, result).is_err() {
                    terminal_error = Some(FrameError::Io(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "writer closed",
                    )));
                    break 'input;
                }
            }
            let message = match framer.read(&mut input) {
                Ok(Some(value)) => value,
                Ok(None) => break,
                Err(error) => {
                    terminal_error = Some(error);
                    break;
                }
            };
            summary.messages_read += 1;
            if cancellation_id(&message).is_some() {
                let id = cancellation_id(&message).expect("checked");
                if let Some(token) = active.get(&id) {
                    token.cancel(
                        CancellationReason::new("MCP peer cancelled request")
                            .expect("static cancellation reason is valid"),
                    );
                } else {
                    summary.unknown_cancellations += 1;
                    bounded_diagnostic(
                        diagnostics,
                        self.options.max_diagnostic_bytes,
                        &format!("mcp-stdio: ignored unknown or late cancellation id={id}"),
                    );
                }
                continue;
            }
            let Some(id) = request_id(&message) else {
                continue;
            };
            if active.contains_key(&id) {
                let error = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"duplicate live request id"}});
                if write_tx.send(error).is_err() {
                    terminal_error = Some(FrameError::Io(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "writer closed",
                    )));
                    break;
                }
                continue;
            }
            if self.options.legacy == LegacyMode::Disabled && is_legacy_lifecycle(&message) {
                let error = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"legacy lifecycle disabled at process construction"}});
                if write_tx.send(error).is_err() {
                    break;
                }
                continue;
            }
            let cancellation = Cancellation::new();
            active.insert(id.clone(), cancellation.clone());
            let wire_id = message["id"].clone();
            let dispatch = Arc::clone(&self.dispatch);
            let done = done_tx.clone();
            let meta = message
                .get("params")
                .and_then(|p| p.get("_meta"))
                .cloned()
                .unwrap_or(Value::Null);
            workers.push(thread::spawn(move || {
                let result = dispatch.dispatch(DispatchCall {
                    message,
                    meta,
                    cancellation,
                });
                let _ = done.send((id, wire_id, result));
            }));
        }
        for token in active.values() {
            token.cancel(
                CancellationReason::new("MCP stdio lifetime ended")
                    .expect("static cancellation reason is valid"),
            );
        }
        for worker in workers {
            let _ = worker.join();
        }
        while let Ok((key, id, result)) = done_rx.try_recv() {
            active.remove(&key);
            if terminal_error.is_none() {
                let _ = enqueue_result(&write_tx, &id, result);
            }
        }
        drop(write_tx);
        summary.messages_written = writer
            .join()
            .map_err(|_| FrameError::Io(std::io::Error::other("writer panicked")))??;
        if let Some(error) = terminal_error {
            return Err(error);
        }
        Ok(summary)
    }
}

fn request_id(value: &Value) -> Option<String> {
    value.get("id").and_then(|id| match id {
        Value::String(s) => Some(format!("s:{s}")),
        Value::Number(n) => Some(format!("n:{n}")),
        _ => None,
    })
}
fn cancellation_id(value: &Value) -> Option<String> {
    (value.get("method")?.as_str()? == "notifications/cancelled")
        .then(|| value.get("params")?.get("requestId"))
        .flatten()
        .and_then(|id| match id {
            Value::String(s) => Some(format!("s:{s}")),
            Value::Number(n) => Some(format!("n:{n}")),
            _ => None,
        })
}
fn is_legacy_lifecycle(value: &Value) -> bool {
    matches!(
        value.get("method").and_then(Value::as_str),
        Some("initialize" | "initialized" | "notifications/initialized" | "shutdown")
    )
}
fn enqueue_result(
    sender: &mpsc::SyncSender<Value>,
    id: &Value,
    result: Result<Vec<Value>, DispatchError>,
) -> Result<(), DispatchError> {
    let values = result.unwrap_or_else(|error| {
        vec![json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":error.0}})]
    });
    for value in values {
        sender
            .send(value)
            .map_err(|_| DispatchError("write queue closed".into()))?;
    }
    Ok(())
}
fn bounded_diagnostic(sink: &mut dyn DiagnosticSink, cap: usize, message: &str) {
    let end = message.floor_char_boundary(message.len().min(cap));
    sink.diagnostic(&message[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{Condvar, Mutex},
        time::Duration,
    };

    #[derive(Default)]
    struct Interleaved {
        calls: Mutex<Vec<(String, Value, Cancellation)>>,
        release: Condvar,
    }
    impl Dispatch for Interleaved {
        fn dispatch(&self, call: DispatchCall) -> Result<Vec<Value>, DispatchError> {
            let id = request_id(&call.message).unwrap();
            self.calls.lock().unwrap().push((
                id.clone(),
                call.meta.clone(),
                call.cancellation.clone(),
            ));
            if id == "s:slow" {
                let guard = self.calls.lock().unwrap();
                let _guard = self
                    .release
                    .wait_timeout_while(guard, Duration::from_millis(150), |_| {
                        !call.cancellation.is_cancelled()
                    })
                    .unwrap()
                    .0;
                // Keep the specimen deterministic: cancellation releases the
                // slow call, but the independent fast call still completes first.
                thread::sleep(Duration::from_millis(10));
            }
            Ok(vec![
                json!({"jsonrpc":"2.0","id":call.message["id"],"result":{"meta":call.meta,"cancelled":call.cancellation.is_cancelled()}}),
            ])
        }
    }

    #[test]
    fn interleaves_principals_out_of_order_and_cancels_only_addressed_request() {
        let dispatch = Arc::new(Interleaved::default());
        let server = StdioServer::new(Arc::clone(&dispatch), ServerOptions::default()).unwrap();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":\"slow\",\"method\":\"tools/call\",\"params\":{\"_meta\":{\"principal\":\"a\",\"version\":\"v1\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"fast\",\"method\":\"tools/call\",\"params\":{\"_meta\":{\"principal\":\"b\",\"version\":\"v2\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":\"slow\"}}\n",
        );
        let output = SharedOutput::default();
        let capture = output.clone();
        let mut diagnostics = Vec::new();
        let summary = server
            .serve(Cursor::new(input.as_bytes()), output, &mut diagnostics)
            .unwrap();
        assert_eq!(summary.messages_written, 2);
        let text = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<_> = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect();
        assert_eq!(lines[0]["id"], "fast");
        let slow = lines.iter().find(|line| line["id"] == "slow").unwrap();
        assert_eq!(slow["result"]["cancelled"], true);
        assert!(
            diagnostics.is_empty(),
            "stdout remains the sole protocol output"
        );
    }

    #[test]
    fn rejects_duplicate_live_ids_and_reports_late_cancellation_only_to_diagnostics() {
        let dispatch = Arc::new(Interleaved::default());
        let server = StdioServer::new(dispatch, ServerOptions::default()).unwrap();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":\"slow\",\"method\":\"x\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"slow\",\"method\":\"x\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":\"missing\"}}\n",
        );
        let output = SharedOutput::default();
        let capture = output.clone();
        let mut diagnostics = Vec::new();
        let summary = server
            .serve(Cursor::new(input.as_bytes()), output, &mut diagnostics)
            .unwrap();
        assert_eq!(summary.unknown_cancellations, 1);
        assert!(
            String::from_utf8(capture.0.lock().unwrap().clone())
                .unwrap()
                .contains("duplicate live request id")
        );
        assert!(
            String::from_utf8(diagnostics)
                .unwrap()
                .contains("unknown or late cancellation")
        );
    }

    #[derive(Clone, Default)]
    struct SharedOutput(Arc<Mutex<Vec<u8>>>);
    impl Write for SharedOutput {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
