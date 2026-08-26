use sim_transport_ports::{
    Datagram, DnsPort, Half, Listener, SocketAddress, SocketPort, Stream, TransportError,
    TransportErrorKind, TransportServices,
};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

pub(super) fn bind_transport() {
    sim_transport_ports::bind_services(TransportServices {
        sockets: Arc::new(TestSockets),
        dns: Arc::new(TestDns),
        ipc: None,
    })
    .expect("test transport binding must remain available");
}

struct TestSockets;

impl SocketPort for TestSockets {
    fn listen_tcp(
        &self,
        address: &SocketAddress,
    ) -> sim_transport_ports::Result<Box<dyn Listener>> {
        let listener = TcpListener::bind(native_address(address)).map_err(native_error)?;
        listener.set_nonblocking(true).map_err(native_error)?;
        Ok(Box::new(TestListener(listener)))
    }

    fn connect_tcp(&self, address: &SocketAddress) -> sim_transport_ports::Result<Box<dyn Stream>> {
        let stream = TcpStream::connect(native_address(address)).map_err(native_error)?;
        stream.set_nodelay(true).map_err(native_error)?;
        Ok(Box::new(TestStream(stream)))
    }

    fn bind_udp(&self, _address: &SocketAddress) -> sim_transport_ports::Result<Box<dyn Datagram>> {
        Err(TransportError::new(
            TransportErrorKind::Unsupported,
            "the agent test adapter does not expose UDP",
        ))
    }
}

struct TestDns;

impl DnsPort for TestDns {
    fn resolve(&self, host: &str, port: u16) -> sim_transport_ports::Result<Vec<SocketAddress>> {
        (host, port)
            .to_socket_addrs()
            .map(|addresses| addresses.map(portable_address).collect())
            .map_err(native_error)
    }
}

struct TestListener(TcpListener);

impl Listener for TestListener {
    fn local_address(&self) -> sim_transport_ports::Result<SocketAddress> {
        self.0
            .local_addr()
            .map(portable_address)
            .map_err(native_error)
    }

    fn accept(&self) -> sim_transport_ports::Result<Option<Box<dyn Stream>>> {
        match self.0.accept() {
            Ok((stream, _)) => Ok(Some(Box::new(TestStream(stream)))),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(native_error(error)),
        }
    }

    fn close(&self) -> sim_transport_ports::Result<()> {
        Ok(())
    }
}

struct TestStream(TcpStream);

impl Read for TestStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(bytes)
    }
}

impl Write for TestStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Stream for TestStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> sim_transport_ports::Result<()> {
        self.0.set_read_timeout(timeout).map_err(native_error)
    }

    fn shutdown(&self, half: Half) -> sim_transport_ports::Result<()> {
        let half = match half {
            Half::Read => Shutdown::Read,
            Half::Write => Shutdown::Write,
            Half::Both => Shutdown::Both,
        };
        self.0.shutdown(half).map_err(native_error)
    }
}

fn native_address(address: &SocketAddress) -> std::net::SocketAddr {
    match address {
        SocketAddress::Ip { address, port } => (*address, *port).into(),
    }
}

fn portable_address(address: std::net::SocketAddr) -> SocketAddress {
    SocketAddress::Ip {
        address: address.ip(),
        port: address.port(),
    }
}

fn native_error(error: std::io::Error) -> TransportError {
    let kind = match error.kind() {
        std::io::ErrorKind::AddrInUse => TransportErrorKind::AddressInUse,
        std::io::ErrorKind::ConnectionRefused => TransportErrorKind::ConnectionRefused,
        std::io::ErrorKind::TimedOut => TransportErrorKind::TimedOut,
        std::io::ErrorKind::WouldBlock => TransportErrorKind::WouldBlock,
        std::io::ErrorKind::NotFound => TransportErrorKind::NotFound,
        _ => TransportErrorKind::ProviderFault,
    };
    TransportError::new(kind, error.to_string())
}

#[cfg(feature = "runner-process")]
pub(super) fn bind_process(cx: &mut sim_kernel::Cx) {
    sim_lib_agent_runner_process::bind_process_port(cx, Arc::new(TestProcess))
        .expect("test process binding must remain available");
}

#[cfg(feature = "runner-process")]
struct TestProcess;

#[cfg(feature = "runner-process")]
impl sim_lib_exec::ProcessPort for TestProcess {
    fn run(
        &self,
        request: &sim_lib_exec::ProcessRequest,
        _cancellation: &sim_lib_exec::ProcessCancellation,
    ) -> sim_lib_exec::ProcessAttempt {
        use sim_lib_exec::{ProcResult, ProcessAttempt, ProcessReceipt, StopReceipt};

        let command = request.program.as_str();
        if command.contains("sleep 1") {
            return ProcessAttempt::StoppedAfterTimeout {
                receipt: StopReceipt {
                    provider: "test/process".into(),
                    elapsed_mono_ns: request.budget.timeout_ms.saturating_mul(1_000_000),
                    cleanup: "modeled child stopped and reaped".into(),
                },
            };
        }
        let (stdout, truncated) = modeled_process(command, request);
        ProcessAttempt::Completed {
            receipt: ProcessReceipt {
                provider: "test/process".into(),
                elapsed_mono_ns: 1,
                result: ProcResult {
                    stdout,
                    stderr: String::new(),
                    exit_code: 0,
                    truncated,
                },
            },
        }
    }
}

#[cfg(feature = "runner-process")]
fn modeled_process(command: &str, request: &sim_lib_exec::ProcessRequest) -> (String, bool) {
    if command.contains("printf 'abcdef'") {
        return ("abc".into(), true);
    }
    if command.contains("printf 'one\\ntwo\\n'") {
        return ("one\ntwo\n".into(), false);
    }
    if command.starts_with("cat > ") && command.contains(" | tee ") {
        return run_recipe_fixture(command, request);
    }
    let start = command
        .find('{')
        .expect("modeled JSON process has an object");
    let end = command
        .rfind('}')
        .expect("modeled JSON process has an object");
    (command[start..=end].to_owned(), false)
}

#[cfg(feature = "runner-process")]
fn run_recipe_fixture(command: &str, request: &sim_lib_exec::ProcessRequest) -> (String, bool) {
    let fields = command.split('\'').collect::<Vec<_>>();
    assert!(
        fields.len() >= 8,
        "unexpected modeled recipe command: {command}"
    );
    let request_path = std::path::Path::new(fields[1]);
    let response = fields[5].to_owned();
    let output_path = std::path::Path::new(fields[7]);
    std::fs::write(
        request_path,
        request.budget.stdin.as_deref().unwrap_or_default(),
    )
    .expect("modeled recipe request must be writable");
    std::fs::write(output_path, &response).expect("modeled recipe output must be writable");
    (response, false)
}
