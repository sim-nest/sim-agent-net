//! SIM's server library: a loadable lib that serves eval and agents over a
//! transport. It exposes sites (local, coroutine, pipeline, loop, fabric),
//! frame routing, REPL drivers, connections, and transports so callers can
//! request evaluation, stream replies, and run agent loops across the runtime.
//!
//! Transports are in-process by default: the default feature set enables no
//! network surface. The network/HTTP transport is the non-default
//! `server-net-http` feature and must be opted into explicitly; the various
//! `trigger-*` features add their respective inbound triggers the same way.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

mod address;
mod citizen;
mod cli;
mod clock;
mod codecio;
mod connection;
#[cfg(feature = "cookbook-web")]
mod cookbook_web;
#[cfg(feature = "cookbook-web")]
mod cookbook_web_json;
mod coroutine;
mod cron;
mod device_edge;
mod dispatch;
mod frame;
mod helpers;
#[cfg(feature = "server-net-http")]
mod http;
#[cfg(feature = "server-net-http")]
mod http_client;
mod isolation;
mod ops;
mod ops_lifecycle;
mod ops_shell;
mod pool;
mod raw_http;
mod realize;
mod registries;
mod repl;
mod router;
mod runtime;
mod server;
mod site;
mod stream_support;
mod transport;
mod trigger;
mod voice;
#[cfg(feature = "wasm")]
mod wasm;

pub use address::ServerAddress;
pub use citizen::{
    ServerAddressDescriptor, ServerFrameDescriptor, server_address_class_symbol,
    server_frame_class_symbol,
};
pub use clock::{DeterministicWallClock, SystemWallClock, WallClock, WallTimestamp};
pub use codecio::{decode_frame_payload, encode_frame_payload};
pub use connection::{Connection, Session};
#[cfg(feature = "cookbook-web")]
pub use cookbook_web::{CookbookWebResponse, CookbookWebState};
pub use coroutine::{Coroutine, CoroutineStatus};
pub use device_edge::{
    DeviceEdgeProfile, DeviceEdgeSession, LedgerRef, LinkKind, register_watch_edge_session,
    watch_route_order,
};
pub use frame::{
    FrameEnvelope, FrameKind, LifecycleCommand, ServerFrame, eval_reply_from_frame,
    eval_request_from_frame, server_frame_from_reply, server_frame_from_request,
};
pub use helpers::parse_duration;
#[cfg(feature = "server-net-http")]
pub use http_client::{HttpGetResponse, http_get};
pub use isolation::{IsolationPolicy, ShareMode};
pub use pool::{WorkerPool, default_worker_pool};
pub use raw_http::{
    BodyLimits, BodyReader, Header, RawConnection, RawHandler, RawHttpServer, RequestHead,
    RequestScope, ResponseHead, ResponseWriter, TrailersPolicy,
};
pub use realize::{realize as realize_site_fragment, realize_stream_events};
pub use registries::{
    AddressResolver, LineDriverFactory, ResolvedAddress, register_address_resolver,
    register_line_driver,
};
pub(crate) use registries::{address_resolvers, line_driver_factories};
pub use repl::{DriverSpec as ReplDriverSpec, LineDriver, ReplOptions, ReplOutput, run_repl};
pub use router::FrameRouter;
pub use runtime::ServerRuntime;
pub use server::{Server, ServerStatus, ThreadMode};
pub use sim_cookbook::EmbeddedDir;
#[cfg(feature = "cookbook-web")]
pub use sim_lib_cookbook::CookbookCapabilityProfile;
pub use site::{
    CoroutineEvalSite, EvalSite, FabricEvalSite, LocalEvalSite, LoopEvalSite, PipelineEvalSite,
    Site, SiteKind, StreamSink,
};
pub use stream_support::{
    BufferedStreamSink, StreamHandle, stream_chunk_frame_from_expr, stream_end_frame,
    stream_frame_from_expr, stream_frame_to_expr, stream_frame_to_value,
};
pub use transport::{
    ConnectionTransport, LocalTransport, LoopbackTransportEndpoint, ServerTransport,
    TcpServerTransport, connect_transport_site, connect_transport_site_with_loopback,
    decode_transport_frame, encode_transport_frame, register_loopback_transport_endpoint,
};
pub use trigger::TriggerHandle;
pub use voice::{
    ASR_TRANSCRIPT_KIND, ASR_TRANSCRIPT_NAMESPACE, MIC_CAPTURE_KIND, MIC_CAPTURE_NAMESPACE,
    ModeledAsrFabric, XR_MIC_CHUNK_KIND, XR_MIC_CHUNK_NAMESPACE, modeled_asr_site,
    modeled_glasses_asr_site,
};
#[cfg(feature = "wasm")]
pub use wasm::register_wasm_region;

use crate::isolation::IsolatedEvalSite;
use cli::{register_server_cli, server_cli_exports};
use dispatch::{register_server_functions, server_exports};
use helpers::{
    connection_arg, coroutine_target_from_value, ensure_installed_codec, keyword, server_arg,
    server_from_value,
};
use ops::{
    server_cancel_coroutine, server_connect, server_coroutine_status, server_lisp, server_loop,
    server_notify, server_pipeline, server_realize, server_receive, server_request,
    server_resume_exprs, server_send, server_start_loop, server_stream, server_stream_next,
    server_yield,
};
use ops_lifecycle::server_start;
use ops_shell::{server_repl, server_trigger, server_trigger_poll, server_wasm_region};
use sim_kernel::{
    AbiVersion, Cx, Lib, LibManifest, LibTarget, Linker, Result, Symbol, Value, Version,
};
use stream_support::stream_handle_arg;
use transport::shutdown_server_transport;

/// The server [`Lib`] implementation, registering the server functions and CLI
/// surface when loaded into a runtime.
pub struct ServerLib;

impl Lib for ServerLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::new("server"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: {
                let mut exports = server_exports();
                exports.extend(server_cli_exports());
                exports
            },
        }
    }

    fn load(&self, cx: &mut sim_kernel::LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        register_server_functions(cx, linker)?;
        register_server_cli(cx, linker)?;
        Ok(())
    }
}

/// Installs the server lib into `cx` exactly once, registering its functions
/// and CLI exports.
pub fn install_server_lib(cx: &mut Cx) -> Result<()> {
    sim_lib_core::install_once(cx, &ServerLib).map(|_| ())
}

pub(crate) fn symbol_list_value(cx: &mut Cx, symbols: &[Symbol]) -> Result<Value> {
    cx.factory().list(
        symbols
            .iter()
            .cloned()
            .map(|symbol| cx.factory().symbol(symbol))
            .collect::<Result<Vec<_>>>()?,
    )
}

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
