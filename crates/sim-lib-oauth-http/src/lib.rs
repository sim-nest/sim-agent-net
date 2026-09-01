//! Policy-bounded OAuth metadata and JWK retrieval over `sim-lib-net-http`.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use serde::Deserialize;
use sim_lib_net_http::{Policy, RedirectPolicy};
use sim_lib_oauth_core::{
    AuthorizationServerMetadata, OAuthError, ProtectedResourceMetadata, Result, ScopeSet,
    SecureUrl, validate_document_size,
};
use sim_lib_oauth_jose::KeyDocument;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

/// Cookbook recipes embedded for discovery and generated documentation.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

include!("documents.rs");

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
