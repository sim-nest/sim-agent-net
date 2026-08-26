//! Deterministic transport fault profiles and a host-free byte stream.
use super::{
    Datagram, DnsPort, Half, IpcAddress, IpcListener, IpcPort, Listener, Result, SocketAddress,
    SocketPort, Stream, TransportError, TransportErrorKind,
};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

/// One deterministic request/response exchange for protocol tests.
#[derive(Clone, Default)]
pub struct ScriptedStreamPort {
    responses: Arc<Mutex<VecDeque<Vec<u8>>>>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ScriptedStreamPort {
    #[must_use]
    pub fn new(responses: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[must_use]
    pub fn requests(&self) -> Vec<Vec<u8>> {
        self.requests.lock().expect("script mutex poisoned").clone()
    }

    #[must_use]
    pub fn services(self: &Arc<Self>) -> super::TransportServices {
        super::TransportServices {
            sockets: self.clone(),
            dns: self.clone(),
            ipc: None,
        }
    }
}

struct ScriptedStream {
    response: VecDeque<u8>,
    request: Vec<u8>,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Drop for ScriptedStream {
    fn drop(&mut self) {
        self.requests
            .lock()
            .expect("script mutex poisoned")
            .push(std::mem::take(&mut self.request));
    }
}

impl Read for ScriptedStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = output.len().min(self.response.len());
        for byte in &mut output[..count] {
            *byte = self.response.pop_front().expect("bounded by response");
        }
        Ok(count)
    }
}

impl Write for ScriptedStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.request.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Stream for ScriptedStream {
    fn set_read_timeout(&self, _: Option<Duration>) -> Result<()> {
        Ok(())
    }
    fn shutdown(&self, _: Half) -> Result<()> {
        Ok(())
    }
}

impl SocketPort for ScriptedStreamPort {
    fn listen_tcp(&self, _: &SocketAddress) -> Result<Box<dyn Listener>> {
        Err(TransportError::new(
            TransportErrorKind::Unsupported,
            "scripted client port cannot listen",
        ))
    }
    fn connect_tcp(&self, _: &SocketAddress) -> Result<Box<dyn Stream>> {
        let response = self
            .responses
            .lock()
            .expect("script mutex poisoned")
            .pop_front()
            .ok_or_else(|| {
                TransportError::new(
                    TransportErrorKind::ConnectionRefused,
                    "script has no response",
                )
            })?;
        Ok(Box::new(ScriptedStream {
            response: response.into(),
            request: Vec::new(),
            requests: self.requests.clone(),
        }))
    }
    fn bind_udp(&self, _: &SocketAddress) -> Result<Box<dyn Datagram>> {
        Err(TransportError::new(
            TransportErrorKind::Unsupported,
            "scripted client port has no datagrams",
        ))
    }
}

impl DnsPort for ScriptedStreamPort {
    fn resolve(&self, _: &str, port: u16) -> Result<Vec<SocketAddress>> {
        Ok(vec![SocketAddress::Ip {
            address: "192.0.2.1".parse().expect("documentation address"),
            port,
        }])
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Profile {
    pub fragment: Option<usize>,
    pub lose_writes: Vec<usize>,
    pub capacity: Option<usize>,
    pub dns_failure: bool,
    pub address_in_use: bool,
    pub cancel_after: Option<usize>,
    pub peer_half_close_after: Option<usize>,
}

#[derive(Default)]
struct State {
    bytes: VecDeque<u8>,
    writes: usize,
    reads: usize,
    read_closed: bool,
    write_closed: bool,
}
/// Host-free stream whose fragmentation, loss, pressure, cancellation, and half-close are scripted.
pub struct ModelStream {
    profile: Profile,
    state: Arc<Mutex<State>>,
}
impl ModelStream {
    #[must_use]
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            state: Arc::new(Mutex::new(State::default())),
        }
    }
}
impl Read for ModelStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut s = self.state.lock().expect("model stream mutex poisoned");
        if self.profile.cancel_after.is_some_and(|n| s.reads >= n) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "scripted cancellation",
            ));
        }
        if s.read_closed
            || self
                .profile
                .peer_half_close_after
                .is_some_and(|n| s.reads >= n)
        {
            return Ok(0);
        }
        let count = out
            .len()
            .min(s.bytes.len())
            .min(self.profile.fragment.unwrap_or(usize::MAX));
        for slot in &mut out[..count] {
            *slot = s.bytes.pop_front().expect("bounded by queue");
        }
        s.reads += 1;
        Ok(count)
    }
}
impl Write for ModelStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut s = self.state.lock().expect("model stream mutex poisoned");
        if s.write_closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "write half closed",
            ));
        }
        let sequence = s.writes;
        s.writes += 1;
        if self.profile.lose_writes.contains(&sequence) {
            return Ok(bytes.len());
        }
        let room = self
            .profile
            .capacity
            .map_or(usize::MAX, |cap| cap.saturating_sub(s.bytes.len()));
        if room == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "scripted backpressure",
            ));
        }
        let count = bytes
            .len()
            .min(room)
            .min(self.profile.fragment.unwrap_or(usize::MAX));
        s.bytes.extend(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Stream for ModelStream {
    fn set_read_timeout(&self, _: Option<Duration>) -> Result<()> {
        Ok(())
    }
    fn shutdown(&self, half: Half) -> Result<()> {
        let mut s = self.state.lock().map_err(|_| {
            TransportError::new(TransportErrorKind::ProviderFault, "model mutex poisoned")
        })?;
        match half {
            Half::Read => s.read_closed = true,
            Half::Write => s.write_closed = true,
            Half::Both => {
                s.read_closed = true;
                s.write_closed = true;
            }
        }
        Ok(())
    }
}

/// Deterministic model implementation of all three transport domains.
type PendingConnections = Arc<Mutex<VecDeque<Box<dyn Stream>>>>;
type ModelNetwork = Arc<Mutex<BTreeMap<String, PendingConnections>>>;
#[derive(Clone)]
pub struct ModelPorts {
    pub profile: Profile,
    pub answers: Vec<SocketAddress>,
    pub ipc: bool,
    network: ModelNetwork,
    next_port: Arc<Mutex<u16>>,
}
impl ModelPorts {
    #[must_use]
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            answers: Vec::new(),
            ipc: false,
            network: Arc::new(Mutex::new(BTreeMap::new())),
            next_port: Arc::new(Mutex::new(40_000)),
        }
    }
}
struct ModelListener {
    address: SocketAddress,
    pending: Arc<Mutex<VecDeque<Box<dyn Stream>>>>,
}
impl Listener for ModelListener {
    fn local_address(&self) -> Result<SocketAddress> {
        Ok(self.address.clone())
    }
    fn accept(&self) -> Result<Option<Box<dyn Stream>>> {
        Ok(self
            .pending
            .lock()
            .expect("model listener mutex poisoned")
            .pop_front())
    }
    fn close(&self) -> Result<()> {
        Ok(())
    }
}
impl SocketPort for ModelPorts {
    fn listen_tcp(&self, address: &SocketAddress) -> Result<Box<dyn Listener>> {
        if self.profile.address_in_use {
            return Err(TransportError::new(
                TransportErrorKind::AddressInUse,
                "scripted address collision",
            ));
        }
        let mut bound = address.clone();
        let SocketAddress::Ip { port, .. } = &mut bound;
        if *port == 0 {
            let mut next = self.next_port.lock().expect("model port mutex poisoned");
            *port = *next;
            *next += 1;
        }
        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let key = format!("{bound:?}");
        let mut network = self.network.lock().expect("model network mutex poisoned");
        if network.contains_key(&key) {
            return Err(TransportError::new(
                TransportErrorKind::AddressInUse,
                "modeled address already bound",
            ));
        }
        network.insert(key, pending.clone());
        Ok(Box::new(ModelListener {
            address: bound,
            pending,
        }))
    }
    fn connect_tcp(&self, address: &SocketAddress) -> Result<Box<dyn Stream>> {
        if self.profile.cancel_after == Some(0) {
            return Err(TransportError::new(
                TransportErrorKind::Cancelled,
                "scripted cancellation",
            ));
        }
        let pending = self
            .network
            .lock()
            .expect("model network mutex poisoned")
            .get(&format!("{address:?}"))
            .cloned()
            .ok_or_else(|| {
                TransportError::new(TransportErrorKind::ConnectionRefused, "no modeled listener")
            })?;
        let (client, server) = duplex();
        pending
            .lock()
            .expect("model listener mutex poisoned")
            .push_back(Box::new(server));
        Ok(Box::new(client))
    }
    fn bind_udp(&self, _: &SocketAddress) -> Result<Box<dyn Datagram>> {
        Err(TransportError::new(
            TransportErrorKind::Unsupported,
            "modeled datagram not selected",
        ))
    }
}
impl DnsPort for ModelPorts {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddress>> {
        if self.profile.dns_failure {
            Err(TransportError::new(
                TransportErrorKind::DnsFailure,
                "scripted DNS failure",
            ))
        } else {
            if !self.answers.is_empty() {
                return Ok(self.answers.clone());
            }
            let address = if host == "localhost" {
                "127.0.0.1".parse().expect("literal")
            } else {
                host.parse().map_err(|_| {
                    TransportError::new(
                        TransportErrorKind::DnsFailure,
                        "model resolves only explicit IP or localhost",
                    )
                })?
            };
            Ok(vec![SocketAddress::Ip { address, port }])
        }
    }
}

#[derive(Default)]
struct Pipe {
    bytes: VecDeque<u8>,
    closed: bool,
}
struct DuplexStream {
    input: Arc<Mutex<Pipe>>,
    output: Arc<Mutex<Pipe>>,
}
fn duplex() -> (DuplexStream, DuplexStream) {
    let a = Arc::new(Mutex::new(Pipe::default()));
    let b = Arc::new(Mutex::new(Pipe::default()));
    (
        DuplexStream {
            input: a.clone(),
            output: b.clone(),
        },
        DuplexStream {
            input: b,
            output: a,
        },
    )
}
impl Read for DuplexStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        for _ in 0..500 {
            let mut p = self.input.lock().expect("duplex mutex poisoned");
            if !p.bytes.is_empty() {
                let n = out.len().min(p.bytes.len());
                for x in &mut out[..n] {
                    *x = p.bytes.pop_front().expect("bounded");
                }
                return Ok(n);
            }
            if p.closed {
                return Ok(0);
            }
            drop(p);
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(io::Error::from(io::ErrorKind::TimedOut))
    }
}
impl Write for DuplexStream {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut p = self.output.lock().expect("duplex mutex poisoned");
        if p.closed {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        p.bytes.extend(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Stream for DuplexStream {
    fn set_read_timeout(&self, _: Option<Duration>) -> Result<()> {
        Ok(())
    }
    fn shutdown(&self, h: Half) -> Result<()> {
        match h {
            Half::Read => self.input.lock().expect("duplex mutex poisoned").closed = true,
            Half::Write => self.output.lock().expect("duplex mutex poisoned").closed = true,
            Half::Both => {
                self.input.lock().expect("duplex mutex poisoned").closed = true;
                self.output.lock().expect("duplex mutex poisoned").closed = true;
            }
        }
        Ok(())
    }
}
struct ModelIpcListener(Profile);
impl IpcListener for ModelIpcListener {
    fn accept(&self) -> Result<Option<Box<dyn Stream>>> {
        Ok(Some(Box::new(ModelStream::new(self.0.clone()))))
    }
    fn close(&self) -> Result<()> {
        Ok(())
    }
}
impl IpcPort for ModelPorts {
    fn listen(&self, _: &IpcAddress) -> Result<Box<dyn IpcListener>> {
        if self.ipc {
            Ok(Box::new(ModelIpcListener(self.profile.clone())))
        } else {
            Err(TransportError::new(
                TransportErrorKind::Unsupported,
                "local IPC service is absent",
            ))
        }
    }
    fn connect(&self, _: &IpcAddress) -> Result<Box<dyn Stream>> {
        if self.ipc {
            Ok(Box::new(ModelStream::new(self.profile.clone())))
        } else {
            Err(TransportError::new(
                TransportErrorKind::Unsupported,
                "local IPC service is absent",
            ))
        }
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
// conformance: bounded provider-neutral transport fault models.
