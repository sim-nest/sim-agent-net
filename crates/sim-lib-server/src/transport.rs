use std::{sync::Arc, thread, time::Duration};

use sim_kernel::{CapabilityName, Cx, Error, Result, Symbol};

use crate::{
    EvalSite, Server, ServerAddress, ServerRuntime, ThreadMode, pool::default_worker_pool,
};

mod backends;
mod framing;
#[cfg(feature = "server-net-http")]
mod http_transport;
#[allow(dead_code)]
pub(crate) mod port_io;
mod site;
mod socket;
#[cfg(feature = "server-net-http")]
mod sse_transport;
#[cfg(test)]
mod tests;
#[cfg(feature = "server-net-http")]
mod ws_transport;

pub use backends::{
    LocalTransport, LoopbackTransportEndpoint, RegistryTransport, WasmConnectionTransport,
};
pub use framing::{decode_transport_frame, encode_transport_frame};
#[cfg(feature = "server-net-http")]
pub use http_transport::{HttpConnectionTransport, HttpServerTransport};
pub use site::TransportEvalSite;
pub use socket::{TcpConnectionTransport, TcpServerTransport};
#[cfg(unix)]
#[allow(unused_imports)]
pub use socket::{UnixConnectionTransport, UnixServerTransport};
#[cfg(feature = "server-net-http")]
pub use sse_transport::{SseConnectionTransport, SseServerTransport};
#[cfg(feature = "server-net-http")]
pub use ws_transport::{WsConnectionTransport, WsServerTransport};

pub(crate) use backends::TransportEndpoint;
use backends::{has_registered_endpoint, register_endpoint, unregister_endpoint};
#[cfg(feature = "server-net-http")]
use framing::io_to_host;
use framing::{
    answer_or_negotiate, error_frame_from_error, is_timeout, read_frame_from, route_frame_bytes,
    update_negotiated_codec_from_reply, write_frame_to,
};

pub(crate) const MAX_TRANSPORT_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const SERVER_CONNECTION_IO_TIMEOUT_MS: u64 = 250;
pub(crate) const DEFAULT_MAX_INFLIGHT_FRAMES: usize = 8;
fn bound_transport_services()
-> std::result::Result<sim_transport_ports::TransportServices, sim_transport_ports::TransportError>
{
    match sim_transport_ports::services() {
        Ok(value) => Ok(value),
        #[cfg(test)]
        Err(_) => {
            let mut model = sim_transport_ports::model::ModelPorts::new(Default::default());
            model.ipc = true;
            let model = Arc::new(model);
            sim_transport_ports::bind_services(sim_transport_ports::TransportServices {
                sockets: model.clone(),
                dns: model.clone(),
                ipc: Some(model),
            })?;
            sim_transport_ports::services()
        }
        #[cfg(not(test))]
        Err(error) => Err(error),
    }
}
pub(crate) const WEBHOOK_SERVE_CAPABILITY: &str = "webhook-serve";
#[cfg(feature = "server-net-http")]
pub(crate) const HTTP_TRANSPORT_PATH: &str = "/sim/frame";
#[cfg(feature = "server-net-http")]
pub(crate) const SSE_TRANSPORT_PATH: &str = "/sim/stream";
#[cfg(feature = "server-net-http")]
pub(crate) const WS_TRANSPORT_PATH: &str = "/sim/ws";

/// Listening side of a transport: binds an address and accepts connections.
pub trait ServerTransport: Send + Sync {
    /// Returns the address this transport is bound to.
    fn address(&self) -> &ServerAddress;
    /// Blocks until a connection arrives and returns it.
    fn accept(&self, cx: &mut Cx) -> Result<Box<dyn ConnectionTransport>>;
    /// Shuts down the listener and releases its resources.
    fn shutdown(&self, cx: &mut Cx) -> Result<()>;

    /// Accepts a connection, returning `None` if `timeout` elapses first.
    fn accept_timeout(
        &self,
        cx: &mut Cx,
        timeout: Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>>;
}

/// One open connection over which server frames are sent and received.
pub trait ConnectionTransport: Send + Sync {
    /// Sends one frame over the connection.
    fn send_frame(&mut self, cx: &mut Cx, frame: crate::ServerFrame) -> Result<()>;
    /// Receives one frame, returning `None` on timeout or end of stream.
    fn recv_frame(
        &mut self,
        cx: &mut Cx,
        timeout: Option<Duration>,
    ) -> Result<Option<crate::ServerFrame>>;
    /// Closes the connection.
    fn close(&mut self, cx: &mut Cx) -> Result<()>;
    /// Returns this connection as `Any` for downcasting.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Serves the connection server-side against `site`.
    ///
    /// The default implementation errors; transports that support server-side
    /// serving override it.
    fn serve_connection(
        &mut self,
        _runtime: &Arc<ServerRuntime>,
        _site: &Arc<dyn EvalSite>,
    ) -> Result<()> {
        Err(Error::Eval(
            "transport does not support server-side serving".to_owned(),
        ))
    }
}

pub fn start_server_transport(server: &Server) -> Result<()> {
    if !server.address().transport_available() {
        return Err(Error::Eval(format!(
            "no transport for address kind {}",
            server.address().kind_symbol()
        )));
    }
    match server.address() {
        ServerAddress::Local | ServerAddress::Any => Ok(()),
        ServerAddress::Tcp { .. }
        | ServerAddress::Unix { .. }
        | ServerAddress::Http { .. }
        | ServerAddress::Sse { .. }
        | ServerAddress::Ws { .. } => {
            let Some(runtime) = server.runtime().cloned() else {
                return Ok(());
            };
            register_endpoint(TransportEndpoint {
                address: server.address().clone(),
                site: server.site().clone(),
            })?;
            let site = server.site().clone();
            match server.thread() {
                ThreadMode::Main => {
                    run_accept_loop(runtime, site);
                    Ok(())
                }
                ThreadMode::Coroutine(_) => Ok(()),
                ThreadMode::Coop | ThreadMode::Spawn | ThreadMode::Pool => {
                    let accept_runtime = runtime.clone();
                    let handle = thread::spawn(move || run_accept_loop(accept_runtime, site));
                    runtime.set_accept_thread(handle)
                }
            }
        }
        ServerAddress::Wasm { region } => require_wasm_region(region),
        _ => register_endpoint(TransportEndpoint {
            address: server.address().clone(),
            site: server.site().clone(),
        }),
    }
}

pub fn shutdown_server_transport(server: &Server) -> Result<()> {
    match server.address() {
        ServerAddress::Local | ServerAddress::Any => Ok(()),
        ServerAddress::Tcp { .. }
        | ServerAddress::Unix { .. }
        | ServerAddress::Http { .. }
        | ServerAddress::Sse { .. }
        | ServerAddress::Ws { .. } => {
            if let Some(runtime) = server.runtime() {
                runtime.begin_stop();
                runtime.join_accept_thread()?;
                runtime.join_worker_threads()?;
                runtime.with_cx(|cx| runtime.transport().shutdown(cx))?;
                runtime.clear_sessions()?;
            }
            unregister_endpoint(server.address())?;
            Ok(())
        }
        ServerAddress::Wasm { .. } => Ok(()),
        _ => unregister_endpoint(server.address()),
    }
}

pub fn require_start_capabilities(cx: &Cx, address: &ServerAddress) -> Result<()> {
    match address {
        ServerAddress::Tcp { .. } | ServerAddress::Unix { .. } => require_network_capability(cx),
        ServerAddress::Http { .. } | ServerAddress::Sse { .. } | ServerAddress::Ws { .. } => {
            require_network_capability(cx)?;
            cx.require(&CapabilityName::new(WEBHOOK_SERVE_CAPABILITY))
        }
        _ => Ok(()),
    }
}

pub fn require_connect_capabilities(cx: &Cx, address: &ServerAddress) -> Result<()> {
    match address {
        ServerAddress::Tcp { .. }
        | ServerAddress::Unix { .. }
        | ServerAddress::Http { .. }
        | ServerAddress::Sse { .. }
        | ServerAddress::Ws { .. } => require_network_capability(cx),
        _ => Ok(()),
    }
}

fn require_network_capability(cx: &Cx) -> Result<()> {
    require_with_aliases(cx, net_http_capability(), net_http_aliases())
}

fn net_http_capability() -> CapabilityName {
    CapabilityName::new("net/http")
}

fn net_http_aliases() -> &'static [&'static str] {
    &["net.http", "net-connect", "network"]
}

fn require_with_aliases(
    cx: &Cx,
    canonical: CapabilityName,
    aliases: &'static [&'static str],
) -> Result<()> {
    if cx.capabilities().contains(&canonical)
        || aliases
            .iter()
            .any(|alias| cx.capabilities().contains(&CapabilityName::new(*alias)))
    {
        Ok(())
    } else {
        Err(Error::CapabilityDenied {
            capability: canonical,
        })
    }
}

/// Connects to `address` and returns the eval site plus negotiated codec.
///
/// Loopback fallback is disabled; see
/// [`connect_transport_site_with_loopback`].
pub fn connect_transport_site(
    cx: &mut Cx,
    address: ServerAddress,
    offered_codecs: Vec<Symbol>,
) -> Result<(Arc<dyn EvalSite>, Symbol)> {
    connect_transport_site_with_loopback(cx, address, offered_codecs, false)
}

/// Connects to `address`, returning the eval site plus negotiated codec.
///
/// When `allow_loopback` is set, a connection that fails may fall back to a
/// registered in-process endpoint for the same address.
pub fn connect_transport_site_with_loopback(
    cx: &mut Cx,
    address: ServerAddress,
    offered_codecs: Vec<Symbol>,
    allow_loopback: bool,
) -> Result<(Arc<dyn EvalSite>, Symbol)> {
    require_connect_capabilities(cx, &address)?;
    TransportEvalSite::connect_with_loopback(cx, address, offered_codecs, allow_loopback)
}

/// Registers `site` as the loopback endpoint for `address`.
///
/// The returned [`LoopbackTransportEndpoint`] unregisters the endpoint when
/// dropped.
pub fn register_loopback_transport_endpoint(
    address: ServerAddress,
    site: Arc<dyn EvalSite>,
) -> Result<LoopbackTransportEndpoint> {
    backends::register_loopback_endpoint(address, site)
}

#[cfg(unix)]
fn open_unix_connection_transport(
    address: &ServerAddress,
    allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    match socket::UnixConnectionTransport::connect(address) {
        Ok(transport) => Ok(Box::new(transport)),
        Err(_error) if allow_loopback && has_registered_endpoint(address)? => {
            Ok(Box::new(RegistryTransport::new(address.clone())))
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(unix))]
fn open_unix_connection_transport(
    _address: &ServerAddress,
    _allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    Err(Error::Eval(
        "unix sockets are not available on this target".to_owned(),
    ))
}

#[cfg(feature = "server-net-http")]
fn open_http_connection_transport(
    address: &ServerAddress,
    allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    match HttpConnectionTransport::connect(address) {
        Ok(transport) => Ok(Box::new(transport)),
        Err(_error) if allow_loopback && has_registered_endpoint(address)? => {
            Ok(Box::new(RegistryTransport::new(address.clone())))
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(feature = "server-net-http"))]
fn open_http_connection_transport(
    _address: &ServerAddress,
    _allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    Err(http_transport_disabled_error())
}

#[cfg(feature = "server-net-http")]
fn open_sse_connection_transport(
    address: &ServerAddress,
    allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    match SseConnectionTransport::connect(address) {
        Ok(transport) => Ok(Box::new(transport)),
        Err(_error) if allow_loopback && has_registered_endpoint(address)? => {
            Ok(Box::new(RegistryTransport::new(address.clone())))
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(feature = "server-net-http"))]
fn open_sse_connection_transport(
    _address: &ServerAddress,
    _allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    Err(http_transport_disabled_error())
}

#[cfg(feature = "server-net-http")]
fn open_ws_connection_transport(
    address: &ServerAddress,
    allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    match WsConnectionTransport::connect(address) {
        Ok(transport) => Ok(Box::new(transport)),
        Err(_error) if allow_loopback && has_registered_endpoint(address)? => {
            Ok(Box::new(RegistryTransport::new(address.clone())))
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(feature = "server-net-http"))]
fn open_ws_connection_transport(
    _address: &ServerAddress,
    _allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    Err(http_transport_disabled_error())
}

fn open_connection_transport(
    address: &ServerAddress,
    allow_loopback: bool,
) -> Result<Box<dyn ConnectionTransport>> {
    match address {
        ServerAddress::Local | ServerAddress::Any => Err(Error::Eval(
            "local addresses require a direct site or server value".to_owned(),
        )),
        ServerAddress::InProcess { .. } | ServerAddress::Coroutine { .. } => {
            Ok(Box::new(RegistryTransport::new(address.clone())))
        }
        ServerAddress::Wasm { .. } => Ok(Box::new(WasmConnectionTransport::connect(address)?)),
        ServerAddress::Http { .. } => open_http_connection_transport(address, allow_loopback),
        ServerAddress::Sse { .. } => open_sse_connection_transport(address, allow_loopback),
        ServerAddress::Ws { .. } => open_ws_connection_transport(address, allow_loopback),
        ServerAddress::Tcp { .. } => match TcpConnectionTransport::connect(address) {
            Ok(transport) => Ok(Box::new(transport)),
            Err(_error) if allow_loopback && has_registered_endpoint(address)? => {
                Ok(Box::new(RegistryTransport::new(address.clone())))
            }
            Err(error) => Err(error),
        },
        ServerAddress::Unix { .. } => open_unix_connection_transport(address, allow_loopback),
        _ => Err(Error::Eval(format!(
            "no connection transport for address kind {}",
            address.kind_symbol()
        ))),
    }
}

#[cfg(unix)]
fn open_unix_server_transport(address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Ok(Some(Arc::new(socket::UnixServerTransport::bind(address)?)))
}

#[cfg(not(unix))]
fn open_unix_server_transport(_address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Err(Error::Eval(
        "unix sockets are not available on this target".to_owned(),
    ))
}

#[cfg(feature = "server-net-http")]
fn open_http_server_transport(address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Ok(Some(Arc::new(HttpServerTransport::bind(address)?)))
}

#[cfg(not(feature = "server-net-http"))]
fn open_http_server_transport(_address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Err(http_transport_disabled_error())
}

#[cfg(feature = "server-net-http")]
fn open_sse_server_transport(address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Ok(Some(Arc::new(SseServerTransport::bind(address)?)))
}

#[cfg(not(feature = "server-net-http"))]
fn open_sse_server_transport(_address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Err(http_transport_disabled_error())
}

#[cfg(feature = "server-net-http")]
fn open_ws_server_transport(address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Ok(Some(Arc::new(WsServerTransport::bind(address)?)))
}

#[cfg(not(feature = "server-net-http"))]
fn open_ws_server_transport(_address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    Err(http_transport_disabled_error())
}

pub fn open_server_transport(address: ServerAddress) -> Result<Option<Arc<dyn ServerTransport>>> {
    match &address {
        ServerAddress::Local | ServerAddress::Any | ServerAddress::Coroutine { .. } => Ok(None),
        ServerAddress::Tcp { .. } => Ok(Some(Arc::new(TcpServerTransport::bind(address)?))),
        ServerAddress::Unix { .. } => open_unix_server_transport(address),
        ServerAddress::InProcess { .. } => Ok(Some(
            Arc::new(RegistryTransport::new(address)) as Arc<dyn ServerTransport>
        )),
        ServerAddress::Wasm { region } => require_wasm_region(region).map(|()| None),
        ServerAddress::Http { .. } => open_http_server_transport(address),
        ServerAddress::Sse { .. } => open_sse_server_transport(address),
        ServerAddress::Ws { .. } => open_ws_server_transport(address),
        _ => Ok(None),
    }
}

pub(crate) fn transport_kind(address: &ServerAddress) -> &'static str {
    match address {
        ServerAddress::InProcess { .. } => "in-proc",
        ServerAddress::Coroutine { .. } => "coroutine",
        ServerAddress::Tcp { .. } => "tcp",
        ServerAddress::Unix { .. } => "unix",
        ServerAddress::Wasm { .. } => "wasm-shmem",
        ServerAddress::Http { .. } => "http",
        ServerAddress::Sse { .. } => "sse",
        ServerAddress::Ws { .. } => "ws",
        _ => "transport",
    }
}

#[cfg(feature = "wasm")]
fn require_wasm_region(region: &str) -> Result<()> {
    let _ = crate::wasm::lookup_wasm_region(region)?;
    Ok(())
}

#[cfg(not(feature = "wasm"))]
fn require_wasm_region(_region: &str) -> Result<()> {
    Err(Error::Eval("server wasm feature disabled".to_owned()))
}

#[cfg(not(feature = "server-net-http"))]
fn http_transport_disabled_error() -> Error {
    Error::Eval("http transport requires the server-net-http feature".to_owned())
}

include!("transport/accept_loop.rs");
