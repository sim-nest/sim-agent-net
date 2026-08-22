//! HTTP-backed model runners for SIM.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

mod client;
mod config;
mod local_service;
mod model_params;
mod probe;
mod provider;
mod redact;
mod runner;
mod stream;

pub use config::ProviderConfig;
pub use local_service::{
    LocalServiceAdapter, LocalServiceEndpoint, register_local_service_families,
};
pub use probe::{
    EndpointCandidate, HttpProbeTransport, ProbeHttpRequest, ProbeHttpResponse, ProbeStatus,
    ProbeTransport, ProviderProbeReport, lemonade_candidates, parse_ollama_tags, probe_provider,
};
pub use provider::{
    ProviderAuth, ProviderProfile, anthropic_profile, lemonade_profile, lm_studio_profile,
    ollama_profile, openai_compatible_profile, openai_profile, provider_profiles,
};
pub use runner::HttpRunner;

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));
