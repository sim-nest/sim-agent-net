//! Effect-free OAuth 2.1 resource/client state and authority facts.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use url::Url;

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

include!("types.rs");
include!("flow.rs");

#[cfg(test)]
mod tests;
