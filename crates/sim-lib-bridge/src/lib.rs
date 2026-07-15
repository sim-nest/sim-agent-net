//! Checked BRIDGE packet runtime for SIM model exchanges.
//!
//! `sim-lib-bridge` is the send and receive guard around the reversible
//! BRIDGE packet codec. It keeps packet validation, capability ceilings,
//! content-addressed request identity, model request construction, response
//! decoding, and runtime exports in library space. The crate targets
//! [`EvalFabric`](sim_kernel::EvalFabric) and the provider-neutral runner
//! contracts; it does not own transports or model providers.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod brief;
mod frontier;
mod materialize;
mod model;
mod receipt;
mod report;
mod runtime;
mod rx;
mod tx;

#[cfg(test)]
mod tests;

pub use brief::{bridge_brief, render_brief_sentences};
pub use frontier::{FrontierMenu, frontier};
pub use materialize::{
    GivenMaterialization, bridge_fetch_capability, bridge_given_materialize_capability,
    fetch_obligation, materialize_given,
};
pub use model::{
    bridge_request_content_key, output_contract_for_packet, terminal_bridge_text,
    terminal_response_packet,
};
pub use receipt::{receipt_packet_for_report, receipt_symbol};
pub use report::{BridgeObligation, BridgeReport};
pub use runtime::{
    BridgeFunction, BridgeFunctionKind, BridgeLib, bridge_brief_symbol, bridge_report_symbol,
    bridge_rx_symbol, bridge_tx_symbol, install_bridge_lib, manifest_name,
};
pub use rx::{bridge_rx, bridge_rx_response, effective_caps, rx_check};
pub use tx::{bridge_tx, prepare_packet, render_model_face, run_bridge};

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
