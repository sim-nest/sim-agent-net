use super::*;
use std::{
    future::Future,
    io::Cursor,
    pin::pin,
    sync::{Condvar, Mutex},
    task::{Context, Poll, Wake, Waker},
};

struct Interleaved {
    calls: Mutex<Vec<(String, Value, Cancellation)>>,
    slow_release: SlowRelease,
}

enum SlowRelease {
    AfterFastOutput(Arc<FastOutput>),
    OnCancellation,
}

#[derive(Default)]
struct FastOutput {
    written: Mutex<bool>,
    release: Condvar,
}

impl Interleaved {
    fn after_fast_output(output: Arc<FastOutput>) -> Self {
        Self {
            calls: Mutex::default(),
            slow_release: SlowRelease::AfterFastOutput(output),
        }
    }

    fn until_cancelled() -> Self {
        Self {
            calls: Mutex::default(),
            slow_release: SlowRelease::OnCancellation,
        }
    }
}

impl Dispatch for Interleaved {
    fn dispatch(&self, call: DispatchCall) -> Result<Vec<Value>, DispatchError> {
        let id = request_id(&call.message).unwrap();
        self.calls
            .lock()
            .unwrap()
            .push((id.clone(), call.meta.clone(), call.cancellation.clone()));
        if id == "s:slow" {
            match &self.slow_release {
                SlowRelease::AfterFastOutput(output) => {
                    let written = output.written.lock().unwrap();
                    let _written = output
                        .release
                        .wait_while(written, |written| !*written)
                        .unwrap();
                }
                SlowRelease::OnCancellation => {
                    wait_for_cancellation(&call.cancellation);
                }
            }
        }
        Ok(vec![
            json!({"jsonrpc":"2.0","id":call.message["id"],"result":{"meta":call.meta,"cancelled":call.cancellation.is_cancelled()}}),
        ])
    }
}

struct ThreadWake(thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn wait_for_cancellation(cancellation: &Cancellation) {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut cancelled = pin!(cancellation.cancelled());
    while cancelled.as_mut().poll(&mut context) == Poll::Pending {
        thread::park();
    }
}

#[test]
fn interleaves_principals_out_of_order_and_cancels_only_addressed_request() {
    let fast_output = Arc::new(FastOutput::default());
    let dispatch = Arc::new(Interleaved::after_fast_output(Arc::clone(&fast_output)));
    let server = StdioServer::new(Arc::clone(&dispatch), ServerOptions::default()).unwrap();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":\"slow\",\"method\":\"tools/call\",\"params\":{\"_meta\":{\"principal\":\"a\",\"version\":\"v1\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":\"fast\",\"method\":\"tools/call\",\"params\":{\"_meta\":{\"principal\":\"b\",\"version\":\"v2\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":\"slow\"}}\n",
    );
    let output = SharedOutput::releasing_slow_after_fast(fast_output);
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
    let dispatch = Arc::new(Interleaved::until_cancelled());
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
struct SharedOutput(Arc<Mutex<Vec<u8>>>, Option<Arc<FastOutput>>);

impl SharedOutput {
    fn releasing_slow_after_fast(fast_output: Arc<FastOutput>) -> Self {
        Self(Arc::default(), Some(fast_output))
    }
}

impl Write for SharedOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut output = self.0.lock().unwrap();
        output.extend_from_slice(buf);
        let fast_written = output
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .any(|line| line["id"] == "fast");
        drop(output);
        if fast_written && let Some(fast_output) = &self.1 {
            *fast_output.written.lock().unwrap() = true;
            fast_output.release.notify_all();
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
