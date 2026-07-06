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
    child.arg("-lc").arg(command);
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

    let writer = thread::spawn(move || -> Result<()> {
        let mut stdin_handle = stdin_handle;
        stdin_handle.write_all(&stdin).map_err(io_error_to_host)
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
    let status = loop {
        {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            if let Some(status) = child.try_wait().map_err(io_error_to_host)? {
                break status;
            }
        }
        if Instant::now() >= deadline {
            let mut child = child
                .lock()
                .map_err(|_| Error::HostError(format!("{label} mutex poisoned")))?;
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = writer.join();
            return Err(Error::Eval(format!(
                "{label} timed out after {}ms",
                max_time.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(10));
    };

    let bytes = rx
        .recv()
        .map_err(|_| Error::HostError(format!("{label} stdout reader failed")))??;
    reader
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdout reader panicked")))?;
    writer
        .join()
        .map_err(|_| Error::HostError(format!("{label} stdin writer panicked")))??;
    if !status.success() {
        return Err(Error::Eval(format!(
            "{label} exited with status {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned())
        )));
    }
    Ok(bytes.into_iter().take(max_output_bytes).collect())
}

pub(super) fn io_error_to_host(err: std::io::Error) -> Error {
    Error::host_io(err)
}
