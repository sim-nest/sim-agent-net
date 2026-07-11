//! Loadable local model runner for SIM placement catalogs.
//!
//! The crate exposes a deterministic local
//! [`ModelRunner`](sim_lib_agent_runner_core::ModelRunner) and a loadable
//! library that registers the runner as a site under `model-site:local`. The
//! default build performs no native inference and needs no model files. The
//! runner card names the local provider, locality, placement key, stream
//! support, and cache support. The `native-inference` feature confines native
//! runtime contact to the `ffi` module, while the `wasm-model` feature loads a
//! model guest through the framed wasm ABI after `ai-runner-local` and
//! `ai-runner-wasm` capability checks.

#![cfg_attr(not(feature = "native-inference"), forbid(unsafe_code))]
#![deny(missing_docs)]
#![allow(deprecated)]

mod backend;
#[cfg(feature = "native-inference")]
mod ffi;
mod register;
#[cfg(feature = "wasm-model")]
mod wasm;

#[cfg(feature = "native-export")]
mod loaders;
#[cfg(feature = "native-export")]
mod native;
#[cfg(feature = "native-export")]
extern crate self as sim;

#[cfg(feature = "native-export")]
use sim_codec_binary as codec_binary;
#[cfg(feature = "native-export")]
use sim_kernel as kernel;
#[cfg(feature = "native-export")]
use sim_macros::{sim_lib, sim_site};

/// Placement key registered by the local model site.
pub const LOCAL_MODEL_SITE_KEY: &str = "model-site:local";
/// Runner symbol advertised by the deterministic local backend.
pub const LOCAL_MODEL_RUNNER: &str = "runner/local-model";
/// Model id advertised by the deterministic local backend.
pub const LOCAL_MODEL_ID: &str = "sim-local-stub";
/// Placement key registered by the local wasm model site.
#[cfg(feature = "wasm-model")]
pub const LOCAL_WASM_MODEL_SITE_KEY: &str = "model-site:local-wasm";
/// Runner symbol advertised by the local wasm backend.
#[cfg(feature = "wasm-model")]
pub const LOCAL_WASM_MODEL_RUNNER: &str = "runner/wasm-local-model";

pub use backend::LocalModelBackend;
pub use register::{LocalModelLib, local_model_site_symbol, realize_site_args};
#[cfg(feature = "wasm-model")]
pub use wasm::{
    WasmModelBackend, WasmModelLib, WasmModelLimits, ai_runner_local_capability,
    ai_runner_wasm_capability, load_wasm_model, local_wasm_model_site_symbol,
};

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
