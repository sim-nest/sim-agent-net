//! Checked BRIDGE packet runtime for SIM model exchanges.
//!
//! `sim-lib-bridge` is the send/receive guard around the reversible BRIDGE packet
//! codec (`sim-codec-bridge`). It runs packets: validation, capability ceilings,
//! content-addressed request identity, model-request construction, response
//! decoding, the shared frontier engine, and the four profile helpers. It targets
//! [`EvalFabric`](sim_kernel::EvalFabric) and the provider-neutral runner
//! contracts; it does not own transports or model providers.
//!
//! # One checker, both directions
//!
//! There is exactly one receive checker (`rx_check`), and transmit runs it on its
//! own output before anything leaves SIM -- so a peer never bounces a packet its
//! author believed well-formed.
//!
//! ```text
//! TX  `bridge_tx` / `run_bridge`:
//!       canonicalize -> stamp cid -> assert_roundtrip -> render_model_face
//!       -> assert_total_ownership -> rx_check (the self-check gate)
//!       -> ModelRequest in EvalRequest.expr -> realize_final
//! RX  `bridge_rx` / `bridge_rx_response`:
//!       terminal_response_packet (LAST content item) -> decode -> rx_check:
//!       move legality + from/to inversion + parent-`Return` reply legality
//!       + per-part shape checks -> BridgeReport (accepted, obligations, repair menus)
//! ```
//!
//! `effective_caps` resolves the header ceiling and every acting step runs under
//! `diminish(current, ceiling)`. `verify_warrant` turns a stale-book packet into a
//! `Fetch` obligation instead of a hard failure or a silent accept.
//!
//! # Profiles (helpers over the one packet)
//!
//! - **ASK** -- `ask_packet` / `run_ask` / `run_ask_with_policy`: a typed model
//!   call whose arguments are fenced data and whose reply is validated against the
//!   declared `Return` shape, with bounded `RepairPolicy` on an `AskFailure`.
//! - **BRIEF** -- `bridge_brief` / `render_brief_sentences`: controlled
//!   instruction frames rendered as fluent cited sentences.
//! - **LOOM** -- `validate_weave` / `weave_row_by_row` over `frontier` /
//!   `FrontierMenu`: model-authored program bodies, checked row by row against a
//!   flat menu, with `LoomObligation`s carrying the valid replacement.
//! - **COLLAB** -- `merge_bridge_replies` / `MergePolicy`: typed reviews, votes,
//!   and patches merged by exact parent cid and target path;
//!   `receipt_packet_for_report` serializes the report lens back onto the wire.
//!
//! `materialize_given` / `fetch_obligation` implement budgeted context with the
//! `Fetch` affordance; `install_bridge_lib` / `BridgeLib` register the runtime
//! verbs (`bridge/tx`, `bridge/rx`, `bridge/ask`, `bridge/run-ask`,
//! `bridge/brief`, ...). `bridge/run-ask` accepts any object exposing
//! [`EvalFabric`](sim_kernel::EvalFabric), a packet expression, and an optional
//! bounded retry count.

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
    AskAttempt, ask_default_codec, ask_packet, ask_packet_with_model_params, run_ask, run_ask_once,
    run_ask_with_policy,
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
    bridge_report_symbol, bridge_run_ask_symbol, bridge_rx_symbol, bridge_tx_symbol,
    install_bridge_lib, manifest_name,
};
pub use rx::{bridge_rx, bridge_rx_response, effective_caps, rx_check};
pub use tx::{bridge_tx, prepare_packet, render_model_face, run_bridge};
pub use warrant::verify_warrant;

/// Cookbook recipes embedded from this crate's `recipes/` directory.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
