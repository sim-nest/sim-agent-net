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
        self.calls
            .lock()
            .unwrap()
            .push((id.clone(), call.meta.clone(), call.cancellation.clone()));
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
