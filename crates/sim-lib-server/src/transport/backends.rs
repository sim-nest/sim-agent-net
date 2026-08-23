use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
};

use sim_kernel::{Cx, Error, Result};

#[cfg(feature = "wasm")]
use sim_wasm_abi::{Frame as WasmFrame, WasmFrameLimits};

#[cfg(feature = "wasm")]
use crate::wasm::lookup_wasm_region;
use crate::{EvalSite, ServerAddress, ServerFrame};

use super::framing::endpoint_key;
use super::{
    ConnectionTransport, ServerTransport, decode_transport_frame, encode_transport_frame,
    route_frame_bytes,
};

#[derive(Clone)]
pub(crate) struct TransportEndpoint {
    pub(crate) address: ServerAddress,
    pub(crate) site: Arc<dyn EvalSite>,
}

#[derive(Default)]
struct EndpointRegistry {
    endpoints: BTreeMap<String, TransportEndpoint>,
}

fn endpoint_registry() -> &'static Mutex<EndpointRegistry> {
    static REGISTRY: OnceLock<Mutex<EndpointRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(EndpointRegistry::default()))
}

pub(crate) fn register_endpoint(endpoint: TransportEndpoint) -> Result<()> {
    let mut registry = endpoint_registry()
        .lock()
        .map_err(|_| Error::HostError("endpoint registry mutex poisoned".to_owned()))?;
    registry
        .endpoints
        .insert(endpoint_key(&endpoint.address), endpoint);
    Ok(())
}

pub(crate) fn has_registered_endpoint(address: &ServerAddress) -> Result<bool> {
    let registry = endpoint_registry()
        .lock()
        .map_err(|_| Error::HostError("endpoint registry mutex poisoned".to_owned()))?;
    Ok(registry.endpoints.contains_key(&endpoint_key(address)))
}

pub(crate) fn unregister_endpoint(address: &ServerAddress) -> Result<()> {
    let mut registry = endpoint_registry()
        .lock()
        .map_err(|_| Error::HostError("endpoint registry mutex poisoned".to_owned()))?;
    registry.endpoints.remove(&endpoint_key(address));
    Ok(())
}

pub(crate) fn lookup_endpoint(address: &ServerAddress) -> Result<TransportEndpoint> {
    let registry = endpoint_registry()
        .lock()
        .map_err(|_| Error::HostError("endpoint registry mutex poisoned".to_owned()))?;
    registry
        .endpoints
        .get(&endpoint_key(address))
        .cloned()
        .ok_or_else(|| {
            Error::Eval(format!(
                "no endpoint registered for {}",
                address.kind_symbol()
            ))
        })
}

/// RAII guard for a registered loopback endpoint.
///
/// Dropping it, or calling [`close`](Self::close), unregisters the endpoint.
pub struct LoopbackTransportEndpoint {
    address: ServerAddress,
}

impl LoopbackTransportEndpoint {
    /// Returns the registered endpoint address.
    pub fn address(&self) -> &ServerAddress {
        &self.address
    }

    /// Unregisters the endpoint explicitly.
    pub fn close(&self) -> Result<()> {
        unregister_endpoint(&self.address)
    }
}

impl Drop for LoopbackTransportEndpoint {
    fn drop(&mut self) {
        let _ = unregister_endpoint(&self.address);
    }
}

pub(crate) fn register_loopback_endpoint(
    address: ServerAddress,
    site: Arc<dyn EvalSite>,
) -> Result<LoopbackTransportEndpoint> {
    register_endpoint(TransportEndpoint {
        address: address.clone(),
        site,
    })?;
    Ok(LoopbackTransportEndpoint { address })
}

#[derive(Clone)]
/// In-process transport that routes frames directly to a local eval site.
///
/// Sending a frame evaluates it against the site and buffers the reply for the
/// next receive.
pub struct LocalTransport {
    address: ServerAddress,
    site: Arc<dyn EvalSite>,
    pending: Arc<Mutex<Option<ServerFrame>>>,
}

impl LocalTransport {
    /// Creates a local transport for `address` backed by `site`.
    pub fn new(address: ServerAddress, site: Arc<dyn EvalSite>) -> Self {
        Self {
            address,
            site,
            pending: Arc::new(Mutex::new(None)),
        }
    }
}

impl ServerTransport for LocalTransport {
    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn accept(&self, _cx: &mut Cx) -> Result<Box<dyn ConnectionTransport>> {
        Ok(Box::new(self.clone()))
    }

    fn shutdown(&self, _cx: &mut Cx) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("local transport mutex poisoned".to_owned()))?;
        *pending = None;
        Ok(())
    }

    fn accept_timeout(
        &self,
        _cx: &mut Cx,
        _timeout: std::time::Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>> {
        Ok(None)
    }
}

impl ConnectionTransport for LocalTransport {
    fn send_frame(&mut self, cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        let reply = route_frame_bytes(cx, &self.site, &encode_transport_frame(&frame)?)?;
        let reply = decode_transport_frame(&reply)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("local transport mutex poisoned".to_owned()))?;
        *pending = Some(reply);
        Ok(())
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Option<ServerFrame>> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("local transport mutex poisoned".to_owned()))?;
        Ok(pending.take())
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("local transport mutex poisoned".to_owned()))?;
        *pending = None;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Clone)]
pub struct RegistryTransport {
    address: ServerAddress,
    pending: Arc<Mutex<Option<ServerFrame>>>,
}

impl RegistryTransport {
    pub fn new(address: ServerAddress) -> Self {
        Self {
            address,
            pending: Arc::new(Mutex::new(None)),
        }
    }
}

impl ServerTransport for RegistryTransport {
    fn address(&self) -> &ServerAddress {
        &self.address
    }

    fn accept(&self, _cx: &mut Cx) -> Result<Box<dyn ConnectionTransport>> {
        let _ = lookup_endpoint(self.address())?;
        Ok(Box::new(Self::new(self.address.clone())))
    }

    fn shutdown(&self, _cx: &mut Cx) -> Result<()> {
        unregister_endpoint(&self.address)
    }

    fn accept_timeout(
        &self,
        _cx: &mut Cx,
        _timeout: std::time::Duration,
    ) -> Result<Option<Box<dyn ConnectionTransport>>> {
        Ok(None)
    }
}

impl ConnectionTransport for RegistryTransport {
    fn send_frame(&mut self, cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        let endpoint = lookup_endpoint(&self.address)?;
        let bytes = encode_transport_frame(&frame)?;
        let reply = route_frame_bytes(cx, &endpoint.site, &bytes)?;
        let reply = decode_transport_frame(&reply)?;
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("registry transport mutex poisoned".to_owned()))?;
        *pending = Some(reply);
        Ok(())
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Option<ServerFrame>> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("registry transport mutex poisoned".to_owned()))?;
        Ok(pending.take())
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| Error::HostError("registry transport mutex poisoned".to_owned()))?;
        *pending = None;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "wasm")]
pub struct WasmConnectionTransport {
    region: String,
    pending: Option<ServerFrame>,
}

#[cfg(feature = "wasm")]
impl WasmConnectionTransport {
    pub fn connect(address: &ServerAddress) -> Result<Self> {
        let ServerAddress::Wasm { region } = address else {
            return Err(Error::Eval(
                "wasm connection transport requires a wasm address".to_owned(),
            ));
        };
        let _ = lookup_wasm_region(region)?;
        Ok(Self {
            region: region.clone(),
            pending: None,
        })
    }
}

#[cfg(feature = "wasm")]
impl ConnectionTransport for WasmConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        let region = lookup_wasm_region(&self.region)?;
        let request = encode_transport_frame(&frame)?;
        enforce_wasm_transport_limit(&request, "wasm frame exceeds transport limit")?;
        let reply = region.runtime.call(
            region.module,
            &sim_kernel::Symbol::qualified("server", "answer"),
            WasmFrame::new(request),
        )?;
        enforce_wasm_frame_limit(&reply, "wasm reply exceeds transport limit")?;
        self.pending = Some(decode_transport_frame(reply.bytes())?);
        Ok(())
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Option<ServerFrame>> {
        Ok(self.pending.take())
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        self.pending = None;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "wasm")]
pub(super) fn enforce_wasm_transport_limit(bytes: &[u8], message: &str) -> Result<()> {
    if bytes.len() > WasmFrameLimits::default().max_frame_bytes {
        return Err(Error::HostError(message.to_owned()));
    }
    Ok(())
}

#[cfg(feature = "wasm")]
pub(super) fn enforce_wasm_frame_limit(frame: &WasmFrame, message: &str) -> Result<()> {
    let frame_ref = frame.as_ref()?;
    if usize::try_from(frame_ref.len).unwrap_or(usize::MAX)
        > WasmFrameLimits::default().max_frame_bytes
    {
        return Err(Error::HostError(message.to_owned()));
    }
    Ok(())
}

#[cfg(not(feature = "wasm"))]
pub struct WasmConnectionTransport;

#[cfg(not(feature = "wasm"))]
impl WasmConnectionTransport {
    pub fn connect(_address: &ServerAddress) -> Result<Self> {
        Err(Error::Eval("server wasm feature disabled".to_owned()))
    }
}

#[cfg(not(feature = "wasm"))]
impl ConnectionTransport for WasmConnectionTransport {
    fn send_frame(&mut self, _cx: &mut Cx, _frame: ServerFrame) -> Result<()> {
        Err(Error::Eval("server wasm feature disabled".to_owned()))
    }

    fn recv_frame(
        &mut self,
        _cx: &mut Cx,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Option<ServerFrame>> {
        Err(Error::Eval("server wasm feature disabled".to_owned()))
    }

    fn close(&mut self, _cx: &mut Cx) -> Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
