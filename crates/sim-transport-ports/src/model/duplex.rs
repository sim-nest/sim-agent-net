use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use crate::{Half, Result, Stream, TransportError, TransportErrorKind};

#[derive(Default)]
struct Pipe {
    bytes: VecDeque<u8>,
    closed: bool,
}

type SharedPipe = Arc<(Mutex<Pipe>, Condvar)>;

pub(super) struct DuplexStream {
    input: SharedPipe,
    output: SharedPipe,
    read_timeout: Mutex<Option<Duration>>,
}

pub(super) fn duplex() -> (DuplexStream, DuplexStream) {
    let a = Arc::new((Mutex::new(Pipe::default()), Condvar::new()));
    let b = Arc::new((Mutex::new(Pipe::default()), Condvar::new()));
    (
        DuplexStream {
            input: a.clone(),
            output: b.clone(),
            read_timeout: Mutex::new(None),
        },
        DuplexStream {
            input: b,
            output: a,
            read_timeout: Mutex::new(None),
        },
    )
}

impl Read for DuplexStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let timeout = *self
            .read_timeout
            .lock()
            .expect("duplex timeout mutex poisoned");
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        let (state, ready) = &*self.input;
        let mut pipe = state.lock().expect("duplex mutex poisoned");
        while pipe.bytes.is_empty() && !pipe.closed {
            if let Some(deadline) = deadline {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                };
                let (next, outcome) = ready
                    .wait_timeout(pipe, remaining)
                    .expect("duplex mutex poisoned");
                pipe = next;
                if outcome.timed_out() && pipe.bytes.is_empty() && !pipe.closed {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
            } else {
                pipe = ready.wait(pipe).expect("duplex mutex poisoned");
            }
        }
        if pipe.bytes.is_empty() {
            return Ok(0);
        }
        let count = out.len().min(pipe.bytes.len());
        for byte in &mut out[..count] {
            *byte = pipe.bytes.pop_front().expect("bounded by queue");
        }
        Ok(count)
    }
}

impl Write for DuplexStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let (state, ready) = &*self.output;
        let mut pipe = state.lock().expect("duplex mutex poisoned");
        if pipe.closed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        pipe.bytes.extend(bytes);
        ready.notify_all();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Stream for DuplexStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        *self.read_timeout.lock().map_err(|_| {
            TransportError::new(
                TransportErrorKind::ProviderFault,
                "duplex timeout mutex poisoned",
            )
        })? = timeout;
        Ok(())
    }

    fn shutdown(&self, half: Half) -> Result<()> {
        match half {
            Half::Read => close_pipe(&self.input),
            Half::Write => close_pipe(&self.output),
            Half::Both => {
                close_pipe(&self.input);
                close_pipe(&self.output);
            }
        }
        Ok(())
    }
}

impl Drop for DuplexStream {
    fn drop(&mut self) {
        close_pipe(&self.output);
    }
}

fn close_pipe(pipe: &SharedPipe) {
    let (state, ready) = &**pipe;
    state.lock().expect("duplex mutex poisoned").closed = true;
    ready.notify_all();
}
