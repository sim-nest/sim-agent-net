use sim_transport_ports::{
    Datagram, DnsPort, Half, Listener, SocketAddress, SocketPort, Stream, TransportError,
    TransportErrorKind, TransportServices, model::ScriptedStreamPort,
};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::Arc,
    time::Duration,
};

pub(super) struct LoopbackOrScriptedPort {
    scripted: Arc<ScriptedStreamPort>,
}

impl LoopbackOrScriptedPort {
    pub(super) fn new(responses: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            scripted: Arc::new(ScriptedStreamPort::new(responses)),
        }
    }

    pub(super) fn services(self: &Arc<Self>) -> TransportServices {
        TransportServices {
            sockets: self.clone(),
            dns: self.clone(),
            ipc: None,
        }
    }

    pub(super) fn requests(&self) -> Vec<Vec<u8>> {
        self.scripted.requests()
    }
}

impl SocketPort for LoopbackOrScriptedPort {
    fn listen_tcp(
        &self,
        address: &SocketAddress,
    ) -> sim_transport_ports::Result<Box<dyn Listener>> {
        self.scripted.listen_tcp(address)
    }

    fn connect_tcp(&self, address: &SocketAddress) -> sim_transport_ports::Result<Box<dyn Stream>> {
        if is_loopback(address) {
            let stream = TcpStream::connect(native_address(address)).map_err(native_error)?;
            stream.set_nodelay(true).map_err(native_error)?;
            Ok(Box::new(LoopbackStream(stream)))
        } else {
            self.scripted.connect_tcp(address)
        }
    }

    fn bind_udp(&self, address: &SocketAddress) -> sim_transport_ports::Result<Box<dyn Datagram>> {
        self.scripted.bind_udp(address)
    }
}

impl DnsPort for LoopbackOrScriptedPort {
    fn resolve(&self, host: &str, port: u16) -> sim_transport_ports::Result<Vec<SocketAddress>> {
        if matches!(host, "localhost" | "127.0.0.1" | "::1") {
            (host, port)
                .to_socket_addrs()
                .map(|addresses| addresses.map(portable_address).collect())
                .map_err(native_error)
        } else {
            self.scripted.resolve(host, port)
        }
    }
}

struct LoopbackStream(TcpStream);

impl Read for LoopbackStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(bytes)
    }
}

impl Write for LoopbackStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Stream for LoopbackStream {
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

fn is_loopback(address: &SocketAddress) -> bool {
    match address {
        SocketAddress::Ip { address, .. } => address.is_loopback(),
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
        std::io::ErrorKind::ConnectionRefused => TransportErrorKind::ConnectionRefused,
        std::io::ErrorKind::TimedOut => TransportErrorKind::TimedOut,
        std::io::ErrorKind::WouldBlock => TransportErrorKind::WouldBlock,
        _ => TransportErrorKind::ProviderFault,
    };
    TransportError::new(kind, error.to_string())
}
