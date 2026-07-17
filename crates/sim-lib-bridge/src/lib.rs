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

mod ask;
mod brief;
mod collab;
mod frontier;
mod loom_validate;
mod loom_woven;
mod materialize;
mod model;
mod parent;
mod receipt;
mod repair;
mod report;
mod runtime;
mod rx;
mod tx;
mod warrant;

#[cfg(test)]
mod tests;

pub use ask::{
    ask_default_codec, ask_packet, ask_packet_with_model_params, run_ask, run_ask_with_policy,
};
pub use brief::{bridge_brief, render_brief_sentences};
pub use collab::{MergePolicy, merge_bridge_replies};
pub use frontier::{FrontierMenu, frontier};
pub use loom_validate::{LoomObligation, next_frontier_menu, validate_weave};
pub use loom_woven::{validate_woven_row, weave_row_by_row};
pub use materialize::{
    GivenMaterialization, bridge_fetch_capability, bridge_given_materialize_capability,
    fetch_obligation, materialize_given,
};
pub use model::{
    bridge_request_content_key, output_contract_for_packet, terminal_bridge_text,
    terminal_response_packet,
};
pub use receipt::{receipt_packet_for_report, receipt_symbol};
pub use repair::{AskFailure, RepairPolicy};
pub use report::{BridgeObligation, BridgeReport};
pub use runtime::{
    BridgeFunction, BridgeFunctionKind, BridgeLib, bridge_ask_symbol, bridge_brief_symbol,
    bridge_report_symbol, bridge_rx_symbol, bridge_tx_symbol, install_bridge_lib, manifest_name,
};
pub use rx::{bridge_rx, bridge_rx_response, effective_caps, rx_check};
pub use tx::{bridge_tx, prepare_packet, render_model_face, run_bridge};
pub use warrant::verify_warrant;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
