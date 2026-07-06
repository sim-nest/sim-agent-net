use sim_kernel::{Error, Result};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

/// Describes a subprocess command invocation and its resource bounds.
#[derive(Clone, Debug)]
pub struct ProcessCommandSpec {
    command: String,
    label: String,
    timeout: Duration,
    max_output_bytes: usize,
}

impl ProcessCommandSpec {
    /// Builds a spec running `command` (labelled `label` for diagnostics),
    /// bounded by `timeout` and `max_output_bytes`.
    pub fn new(
        command: impl Into<String>,
        label: impl Into<String>,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Self {
        Self {
            command: command.into(),
            label: label.into(),
            timeout,
            max_output_bytes,
        }
    }
}

/// Runs the command in `spec`, feeding `stdin`, and returns its captured stdout.
pub fn run_process_command(spec: &ProcessCommandSpec, stdin: Vec<u8>) -> Result<Vec<u8>> {
    run_command(
        &spec.command,
        stdin,
        &spec.label,
        spec.timeout,
        spec.max_output_bytes,
    )
}

/// Runs the command in `spec`, feeding `stdin`, invoking `on_line` for each
/// stdout line as it arrives, and returns the full captured stdout.
pub fn stream_process_command_lines(
    spec: &ProcessCommandSpec,
    stdin: Vec<u8>,
    on_line: impl FnMut(&[u8]) -> Result<()>,
) -> Result<Vec<u8>> {
    stream_command_lines(
        &spec.command,
        stdin,
        &spec.label,
        spec.timeout,
        spec.max_output_bytes,
        on_line,
    )
}

pub(super) fn shell_child(command: &str) -> Command {
    let mut child = Command::new("/bin/sh");
    // `-c` (not `-lc`): a model call must not source the host's login profile
    // files; it runs only the command it was given.
    child.arg("-c").arg(command);
    child
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    child
}

pub(super) fn capture_child_output(
    mut child: Child,
    stdin: Vec<u8>,
    label: &str,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<Vec<u8>> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::HostError(format!("{label} stdout was not captured")))?;
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| Error::HostError(format!("{label} stdin was not captured")))?;
    let child = Arc::new(Mutex::new(child));

    // Keep the raw io error so the join site can tell a benign EPIPE/WriteZero
    // (the child stopped reading early) from a real write failure.
    let writer = thread::spawn(move || -> std::io::Result<()> {
        let mut stdin_handle = stdin_handle;
        stdin_handle.write_all(&stdin)
    });
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = stdout;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let remaining = max_output_bytes
                        .saturating_add(1)
                        .saturating_sub(bytes.len());
                    if remaining == 0 {
                        break;
                    }
                    let take = remaining.min(read);
                    bytes.extend_from_slice(&chunk[..take]);
                    if bytes.len() > max_output_bytes {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(io_error_to_host(err)));
                    return;
                }
            }
        }
        let _ = tx.send(Ok(bytes));
    });

    // Bound BOTH the child exit and the stdout drain by the deadline. A
    // backgrounded grandchild can hold the stdout pipe open after the shell
    // exits, so an unbounded `rx.recv()` here would hang forever despite the
    // timeout; pace the wait with `recv_timeout` and kill on expiry.
    let deadline = Instant::now() + timeout;
    let mut status: Option<std::process::ExitStatus> = None;
    let mut captured: Option<Result<Vec<u8>>> = None;
    loop {
        if status.is_none() {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            status = child.try_wait().map_err(io_error_to_host)?;
        }
        if captured.is_none() {
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(message) => captured = Some(message),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => captured = Some(Ok(Vec::new())),
            }
        } else if status.is_none() {
            thread::sleep(Duration::from_millis(10));
        }
        if status.is_some() && captured.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            let _ = child.kill();
            let _ = child.wait();
            // Do not join the reader/writer here: either may still be blocked on
            // a pipe a grandchild holds open, which would re-introduce the hang.
            return Err(Error::Eval(format!(
                "{label} timed out after {}ms",
                timeout.as_millis()
            )));
        }
    }

    let status =
        status.ok_or_else(|| Error::HostError(format!("{label} status was not captured")))?;
    let bytes =
        captured.ok_or_else(|| Error::HostError(format!("{label} stdout reader failed")))??;
    reader
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdout reader panicked")))?;
    let writer_outcome = writer
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdin writer panicked")))?;
    if bytes.len() > max_output_bytes {
        return Err(Error::Eval(format!(
            "{label} exceeded max output bytes {max_output_bytes}"
        )));
    }
    if !status.success() {
        // The exit status is the real diagnostic for an early-exiting command;
        // a stdin EPIPE caused by that early exit must not mask it.
        return Err(Error::Eval(format!(
            "{label} exited with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned())
        )));
    }
    // The child succeeded. Surface a stdin write error only when it is not a
    // benign pipe closure from the child closing stdin after reading enough.
    if let Err(err) = writer_outcome
        && !is_benign_stdin_pipe_error(&err)
    {
        return Err(io_error_to_host(err));
    }
    Ok(bytes)
}

/// Whether a stdin-writer error is the benign "child stopped reading" closure
/// (EPIPE / zero-length write) rather than a real I/O failure.
fn is_benign_stdin_pipe_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::WriteZero
    )
}

pub(super) fn run_command(
    command: &str,
    stdin: Vec<u8>,
    label: &str,
    timeout: Duration,
    max_output_bytes: usize,
) -> Result<Vec<u8>> {
    let child = shell_child(command).spawn().map_err(io_error_to_host)?;
    capture_child_output(child, stdin, label, timeout, max_output_bytes)
}

pub(super) fn stream_command_lines(
    command: &str,
    stdin: Vec<u8>,
    label: &str,
    timeout: Duration,
    max_output_bytes: usize,
    mut on_line: impl FnMut(&[u8]) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut child = shell_child(command).spawn().map_err(io_error_to_host)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::HostError(format!("{label} stdout was not captured")))?;
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| Error::HostError(format!("{label} stdin was not captured")))?;
    let child = Arc::new(Mutex::new(child));
    // Return the raw io error so the join site can tell a benign EPIPE/WriteZero
    // (the child stopped reading stdin early -- e.g. a command that ignores its
    // input and exits) from a real write failure, matching capture_child_output.
    let writer = thread::spawn(move || -> std::io::Result<()> {
        let mut stdin_handle = stdin_handle;
        stdin_handle.write_all(&stdin)
    });
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(Ok(line)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(io_error_to_host(err)));
                    return;
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let mut status = None;
    let mut reader_done = false;
    while !reader_done || status.is_none() {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Ok(line)) => {
                if bytes.len().saturating_add(line.len()) > max_output_bytes {
                    kill_child(&child);
                    // Do not join reader/writer: a backgrounded grandchild can
                    // keep the stdout/stdin pipes open after the direct child is
                    // killed, so joining would block past the bound. Mirror the
                    // non-streaming timeout path in capture_child_output.
                    return Err(Error::Eval(format!(
                        "{label} exceeded max output bytes {max_output_bytes}"
                    )));
                }
                on_line(&line)?;
                bytes.extend_from_slice(&line);
            }
            Ok(Err(err)) => {
                kill_child(&child);
                // Do not join reader/writer: a grandchild may still hold the
                // pipes open, which would re-introduce the hang.
                return Err(err);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                reader_done = true;
            }
        }
        if status.is_none() {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            status = child.try_wait().map_err(io_error_to_host)?;
        }
        if Instant::now() >= deadline {
            kill_child(&child);
            // Do not join reader/writer here: either may still be blocked on a
            // pipe a grandchild holds open, which would re-introduce the hang.
            // Mirror the non-streaming timeout path in capture_child_output.
            return Err(Error::Eval(format!(
                "{label} timed out after {}ms",
                timeout.as_millis()
            )));
        }
    }
    reader
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdout reader panicked")))?;
    let writer_outcome = writer
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdin writer panicked")))?;
    let status =
        status.ok_or_else(|| Error::HostError(format!("{label} status was not captured")))?;
    if !status.success() {
        // The exit status is the real diagnostic for an early-exiting command;
        // a stdin EPIPE caused by that early exit must not mask it.
        return Err(Error::Eval(format!(
            "{label} exited with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned())
        )));
    }
    // The child succeeded. Surface a stdin write error only when it is not a
    // benign pipe closure from the child closing stdin after reading enough.
    if let Err(err) = writer_outcome
        && !is_benign_stdin_pipe_error(&err)
    {
        return Err(io_error_to_host(err));
    }
    Ok(bytes)
}

fn kill_child(child: &Arc<Mutex<Child>>) {
    if let Ok(mut child) = child.lock() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(super) fn io_error_to_host(err: std::io::Error) -> Error {
    Error::HostError(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProcessProtocol, ProcessRunner, effects::host_process_capability};
    use sim_kernel::{CapabilityName, Cx, DefaultFactory, Expr, NoopEvalPolicy, Symbol};
    use sim_lib_agent_runner_core::ModelRequest;

    #[test]
    fn denied_host_process_refuses_before_spawn() {
        let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
        let runner = ProcessRunner::new(
            Symbol::new("p"),
            "m",
            "echo should-not-run",
            ProcessProtocol::LineText,
            Duration::from_secs(5),
            1024,
        );
        let request = ModelRequest::new(Expr::String("hi".to_owned()), Vec::new());

        let err = crate::effects::resolve_process_effect(&runner, &mut cx, request, |_, _| {
            panic!("subprocess spawned despite denied host-process capability")
        })
        .expect_err("denied capability must refuse before spawn");

        assert!(matches!(
            err,
            Error::CapabilityDenied { capability }
                if capability == host_process_capability()
        ));
    }

    #[test]
    fn granted_host_process_allows_spawn() {
        let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
        cx.grant(CapabilityName::new("host.process"));
        // With the capability granted, the effect-path `cx.require` passes, so a
        // real `infer` would proceed to spawn; exercise the spawn directly.
        assert!(cx.require(&host_process_capability()).is_ok());
        let bytes = run_command(
            "printf ok",
            Vec::new(),
            "test",
            Duration::from_secs(5),
            1024,
        )
        .expect("granted command runs");
        assert_eq!(bytes, b"ok");
    }

    #[test]
    fn streaming_tolerates_benign_stdin_epipe_from_early_exit() {
        // A streamed command that ignores stdin and exits closes its stdin
        // read-end before the writer drains the payload. A 1 MiB payload exceeds
        // the OS pipe buffer, so write_all is still blocked when the child exits,
        // making the resulting EPIPE deterministic. It is benign and must NOT
        // fail the call (regression: the streaming path used to map+propagate it,
        // which flaked under load while the non-streaming path tolerated it).
        let mut lines = Vec::new();
        let bytes = stream_command_lines(
            "printf 'one\\ntwo\\n'",
            vec![b'x'; 1024 * 1024],
            "test",
            Duration::from_secs(5),
            1024,
            |line| {
                lines.push(
                    String::from_utf8_lossy(line)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned(),
                );
                Ok(())
            },
        )
        .expect("a stdin-ignoring streamed command must not fail on benign EPIPE");
        assert_eq!(lines, vec!["one".to_owned(), "two".to_owned()]);
        assert_eq!(bytes, b"one\ntwo\n");
    }

    #[test]
    fn backgrounded_grandchild_returns_at_timeout_not_hang() {
        let start = Instant::now();
        let err = run_command(
            "sleep 0.01; (sleep 60 &)",
            Vec::new(),
            "test",
            Duration::from_millis(300),
            4096,
        )
        .expect_err("a grandchild holding stdout must not hang past the deadline");

        assert!(matches!(err, Error::Eval(message) if message.contains("timed out")));
        // Far below the 60s grandchild sleep: the deadline, not the grandchild,
        // bounded the wait.
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn streaming_backgrounded_grandchild_returns_at_timeout_not_hang() {
        // A shell grandchild inherits stdout and keeps it open after the direct
        // child exits. The streaming reader thread stays blocked on that pipe, so
        // joining it at the deadline would hang forever; the timeout branch must
        // return without joining (regression for the streaming path).
        let start = Instant::now();
        let mut lines = Vec::new();
        let err = stream_command_lines(
            "printf 'ready\\n'; (sleep 60 &); sleep 60",
            Vec::new(),
            "test",
            Duration::from_millis(300),
            4096,
            |line| {
                lines.push(
                    String::from_utf8_lossy(line)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned(),
                );
                Ok(())
            },
        )
        .expect_err("a streaming grandchild must not hang past the deadline");

        assert!(matches!(err, Error::Eval(message) if message.contains("timed out")));
        assert!(lines.contains(&"ready".to_owned()));
        // Far below the 60s grandchild sleep: the deadline bounded the wait.
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn streaming_max_output_with_grandchild_returns_not_hang() {
        // The max-output branch also kills the child then used to join; a
        // grandchild holding stdout would hang it. Emit past the cap while a
        // grandchild keeps the pipe open and confirm a prompt bounded error.
        let start = Instant::now();
        let err = stream_command_lines(
            "(sleep 60 &); printf 'aaaaaaaaaa\\nbbbbbbbbbb\\n'; sleep 60",
            Vec::new(),
            "test",
            Duration::from_secs(5),
            4,
            |_line| Ok(()),
        )
        .expect_err("exceeding max output must return, not hang on a grandchild");

        assert!(matches!(err, Error::Eval(message) if message.contains("max output bytes")));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn early_exit_reports_status_not_stdin_pipe_error() {
        // `head -c1` consumes one byte then the shell exits 3, dropping the read
        // end of stdin while the writer still has ~1 MiB to push (EPIPE).
        let stdin = vec![b'x'; 1 << 20];
        let err = run_command(
            "head -c1 >/dev/null; exit 3",
            stdin,
            "test",
            Duration::from_secs(5),
            1 << 20,
        )
        .expect_err("a non-zero exit must surface as an error");

        match err {
            Error::Eval(message) => {
                assert!(
                    message.contains("exited with status 3"),
                    "expected exit-3 status, got: {message}"
                );
                assert!(
                    !message.to_lowercase().contains("pipe"),
                    "stdin pipe error masked the real exit status: {message}"
                );
            }
            other => panic!("expected an Eval status error, got: {other}"),
        }
    }
}
