#![forbid(unsafe_code)]
//! Narrow, host-neutral ports at SIM's connection, listener, resolver, and IPC boundary.
//!
//! Protocol framing and session policy deliberately do not live here. Platform capsules
//! implement these byte-oriented services; deterministic tests use [`model`].

use std::{
    fmt,
    io::{Read, Write},
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};

/// A provider-neutral network address. Name resolution is deliberately separate.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SocketAddress {
    Ip {
        address: std::net::IpAddr,
        port: u16,
    },
}

/// Local IPC identities are platform-specific and never coerced to a common path contract.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IpcAddress {
    UnixPath(std::path::PathBuf),
    WindowsPipe(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Half {
    Read,
    Write,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportErrorKind {
    Unsupported,
    NotFound,
    DnsFailure,
    AddressInUse,
    ConnectionRefused,
    TimedOut,
    WouldBlock,
    Cancelled,
    Closed,
    InvalidAddress,
    ProviderFault,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub detail: String,
}
impl TransportError {
    #[must_use]
    pub fn new(kind: TransportErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}
impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}
impl std::error::Error for TransportError {}
pub type Result<T> = std::result::Result<T, TransportError>;

pub trait Stream: Read + Write + Send + Sync {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<()>;
    fn shutdown(&self, half: Half) -> Result<()>;
}
pub trait Listener: Send + Sync {
    fn local_address(&self) -> Result<SocketAddress>;
    fn accept(&self) -> Result<Option<Box<dyn Stream>>>;
    fn close(&self) -> Result<()>;
}
pub trait Datagram: Send {
    fn local_address(&self) -> Result<SocketAddress>;
    fn send_to(&mut self, bytes: &[u8], target: &SocketAddress) -> Result<usize>;
    fn recv_from(&mut self, bytes: &mut [u8]) -> Result<Option<(usize, SocketAddress)>>;
    fn close(&self) -> Result<()>;
}
/// TCP/UDP realization only. Host names must pass through [`DnsPort`].
pub trait SocketPort: Send + Sync {
    fn listen_tcp(&self, address: &SocketAddress) -> Result<Box<dyn Listener>>;
    fn connect_tcp(&self, address: &SocketAddress) -> Result<Box<dyn Stream>>;
    fn bind_udp(&self, address: &SocketAddress) -> Result<Box<dyn Datagram>>;
}
pub trait DnsPort: Send + Sync {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddress>>;
}
pub trait IpcListener: Send + Sync {
    fn accept(&self) -> Result<Option<Box<dyn Stream>>>;
    fn close(&self) -> Result<()>;
}
/// Optional local IPC service. A capsule supports only its native address variants.
pub trait IpcPort: Send + Sync {
    fn listen(&self, address: &IpcAddress) -> Result<Box<dyn IpcListener>>;
    fn connect(&self, address: &IpcAddress) -> Result<Box<dyn Stream>>;
}

/// Explicit platform binding consumed by protocol crates. IPC remains optional.
#[derive(Clone)]
pub struct TransportServices {
    pub sockets: Arc<dyn SocketPort>,
    pub dns: Arc<dyn DnsPort>,
    pub ipc: Option<Arc<dyn IpcPort>>,
}

fn binding() -> &'static RwLock<Option<TransportServices>> {
    static BINDING: OnceLock<RwLock<Option<TransportServices>>> = OnceLock::new();
    BINDING.get_or_init(|| RwLock::new(None))
}

/// Installs the active capsule's transport realization.
pub fn bind_services(services: TransportServices) -> Result<()> {
    *binding().write().map_err(|_| {
        TransportError::new(
            TransportErrorKind::ProviderFault,
            "transport binding lock poisoned",
        )
    })? = Some(services);
    Ok(())
}

/// Returns explicitly bound services; there is no ambient native fallback.
pub fn services() -> Result<TransportServices> {
    binding()
        .read()
        .map_err(|_| {
            TransportError::new(
                TransportErrorKind::ProviderFault,
                "transport binding lock poisoned",
            )
        })?
        .clone()
        .ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::Unsupported,
                "no platform transport services are bound",
            )
        })
}
pub mod model;
