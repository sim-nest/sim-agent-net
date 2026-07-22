use sim_kernel::{Error, Result};
use std::{
    io::{Read, Write},
    process::{Child, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

pub(super) fn shell_child(command: &str) -> std::process::Command {
    let mut child = std::process::Command::new("/bin/sh");
    // Do not use a login shell: model- or tool-provided commands must not
    // source host profile state before running.
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
    max_time: Duration,
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

    let (writer_tx, writer_rx) = mpsc::channel();
    let writer = thread::spawn(move || {
        let mut stdin_handle = stdin_handle;
        let _ = writer_tx.send(stdin_handle.write_all(&stdin));
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

    let deadline = Instant::now() + max_time;
    let mut status = None;
    let mut captured = None;
    let mut writer_outcome = None;
    loop {
        if status.is_none() {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            status = child.try_wait().map_err(io_error_to_host)?;
        }
        if captured.is_none() {
            match rx.try_recv() {
                Ok(message) => captured = Some(message),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => captured = Some(Ok(Vec::new())),
            }
        }
        if writer_outcome.is_none() {
            match writer_rx.try_recv() {
                Ok(message) => writer_outcome = Some(message),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => writer_outcome = Some(Ok(())),
            }
        }
        if status.is_some() && captured.is_some() && writer_outcome.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            let _ = child.kill();
            let _ = child.wait();
            // Do not join helper threads on deadline: a grandchild can keep a
            // pipe open after the shell exits or is killed.
            return Err(Error::Eval(format!(
                "{label} timed out after {}ms",
                max_time.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(10));
    }

    let status =
        status.ok_or_else(|| Error::HostError(format!("{label} status was not captured")))?;
    let bytes =
        captured.ok_or_else(|| Error::HostError(format!("{label} stdout reader failed")))??;
    let writer_outcome =
        writer_outcome.ok_or_else(|| Error::HostError(format!("{label} stdin writer failed")))?;
    reader
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdout reader panicked")))?;
    writer
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdin writer panicked")))?;
    if !status.success() {
        return Err(Error::Eval(format!(
            "{label} exited with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned())
        )));
    }
    if let Err(err) = writer_outcome
        && !is_benign_stdin_pipe_error(&err)
    {
        return Err(io_error_to_host(err));
    }
    Ok(bytes.into_iter().take(max_output_bytes).collect())
}

fn is_benign_stdin_pipe_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::WriteZero
    )
}

pub(super) fn io_error_to_host(err: std::io::Error) -> Error {
    Error::host_io(err)
}

#[cfg(test)]
mod tests {
    use super::{capture_child_output, shell_child};
    use sim_kernel::Error;
    use std::time::{Duration, Instant};

    #[test]
    fn backgrounded_grandchild_stdout_is_bounded_by_timeout() {
        let start = Instant::now();
        let child = shell_child("sleep 0.01; (sleep 60 &)").spawn().unwrap();
        let err = capture_child_output(child, Vec::new(), "test", Duration::from_millis(300), 4096)
            .expect_err("grandchild-held stdout must not hang");

        assert!(matches!(err, Error::Eval(message) if message.contains("timed out")));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
