#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Content-addressed distributed evaluation for remote stream realization.
//!
//! This crate is the EvalFabric / realize transport layer for SIM streams: the
//! location-transparent eval surface that server and agent code target to place
//! work and exchange streams, rather than reaching for transport-specific APIs.
//! It turns `StreamValue` packet spines into kernel chunk events or existing
//! server stream frames, and turns event or frame buffers back into lazy stream
//! spines. It stays in library space: it does not add server frame kinds or
//! kernel object hooks.
//!
//! Content-addressed evaluation treats a fleet as a store of immutable eval
//! replies. [`ContentServeFabric`] answers a request only if this node already
//! holds its [`ContentKey`]. [`ContentAddressedFabric`] presents local and peer
//! cassettes as one [`EvalFabric`]: it checks the local [`EvalCassette`], asks
//! peers that hold the content id, computes on a home site when no holder has
//! the reply, and records every successful reply so this node becomes a holder
//! too. There is no routing table; the lookup question is simply "who holds
//! content id X." [`HoldingIndex`] is a grow-only gossip index of holders.
//! Because content never mutates, a holder fact is never invalidated and the
//! index needs no consensus. Recovery is [`EvalCassette::from_ledger`] replay.
//! The `FleetRelayFabric` design names the explicit-routing alternative for
//! small, statically known fleets.
//!
//! The surface has four families:
//!
//! - Control plane: [`StreamControl`] and its frame round-trip carry open, next,
//!   push, close, cancel, stats, metadata, and fault actions between peers.
//! - Frame codec: [`stream_to_frames`] and the `stream_frames_to_*` family
//!   encode a stream into bounded server frames and decode them back, applying
//!   [`StreamFrameLimits`].
//! - Placement: [`ServerPlacementRequest`] and [`server_placement_stream_frames`]
//!   place a fragment on a site under [`PlacementResourceLimits`], with sensitive
//!   inputs redacted by [`redact_placement_expr`].
//! - Realize requests: [`stream_realize_request`] and [`realize_stream_events`]
//!   bridge a stream to the realize event surface.
//! - Relay surfaces: [`RelayFabric`] carries a surface session as a remote
//!   [`EvalFabric`] node. [`ContentKey`] gives each stable [`EvalRequest`] a
//!   deterministic identity, [`EvalCassette`] stores successful replies in an
//!   effect-ledger-backed cache, [`LedgeredRelayFabric`] adds cache-first
//!   cassette replay and write-through around any inner fabric, and
//!   [`ContentAddressedFabric`] resolves work from any holder by content id.
//!   Capability policy stays with the inner fabric and is not weakened by
//!   caching.
//!
//! [`EvalFabric`]: sim_kernel::EvalFabric
//! [`EvalRequest`]: sim_kernel::EvalRequest

mod cassette;
mod content_key;
mod control;
mod events;
mod frames;
mod holders;
mod ledgered_relay;
mod placement;
mod placement_report;
mod placement_security;
mod relay;
mod request;
mod store;

pub use cassette::{EffectLedgerCassette, EvalCassette, EvalCassetteLedger};
pub use content_key::ContentKey;
pub use control::{
    StreamControl, stream_control_cancel_symbol, stream_control_close_symbol,
    stream_control_fault_symbol, stream_control_frame_from_control, stream_control_from_frame,
    stream_control_metadata_symbol, stream_control_next_symbol, stream_control_open_symbol,
    stream_control_operation_symbols, stream_control_push_symbol,
    stream_control_required_capability, stream_control_stats_symbol,
};
pub use events::{
    event_buffer_to_stream, expr_chunk_event, refused_profile_diagnostic_kind,
    remote_error_diagnostic_kind, stream_limit_diagnostic_kind,
};
pub use frames::{
    StreamFrameLimits, cassette_to_stream_frames, expr_to_stream_chunk_frame,
    remote_frame_to_event, stream_chunk_frame_to_expr, stream_frames_to_cassette,
    stream_frames_to_events, stream_frames_to_stream, stream_to_frames,
    stream_to_frames_with_envelope, stream_to_frames_with_limits, stream_to_frames_with_profile,
};
pub use holders::HoldingIndex;
pub use ledgered_relay::{LedgeredRelayFabric, ledgered_relay_fabric_value};
pub use placement::{
    ServerPlacementRequest, server_buffered_preview_profile,
    server_placement_frame_operation_symbols, server_placement_refusal_diagnostic_kind,
    server_placement_refusal_symbol, server_placement_request_symbol,
    server_placement_result_symbol, server_placement_stream_frames, server_render_return_profile,
    server_site_symbol,
};
pub use placement_security::{
    PlacementCapability, PlacementResourceLimits, placement_capability_names,
    placement_host_device_capability, placement_remote_render_capability,
    placement_run_node_on_lan_peer_capability, placement_run_node_on_server_capability,
    redact_placement_expr,
};
pub use relay::{RelayFabric, RelayStatus};
pub use request::{realize_placed_stream_events, realize_stream_events, stream_realize_request};
pub use store::{ContentAddressedFabric, ContentPeer, ContentServeFabric, HeldContent};

#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod placement_tests;
#[cfg(test)]
mod store_tests;
/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
