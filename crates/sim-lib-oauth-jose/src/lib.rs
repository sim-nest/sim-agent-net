//! Local JWT/JWK-set verification with no I/O and no algorithm inference.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::signature;
use serde::Deserialize;
use sim_lib_oauth_core::{
    AccessTokenVerifier, OAuthError, Result, ScopeSet, Secret, SecureUrl, VerifiedPrincipal,
};
use std::collections::{BTreeSet, HashSet};

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

include!("verifier.rs");

#[cfg(test)]
mod tests;
