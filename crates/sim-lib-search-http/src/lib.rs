//! Provider-neutral HTTP execution for [`SearchWireCodec`] implementations.
//!
//! This crate owns policy and effects, never provider syntax. Configuration is
//! supplied as a checked [`ConfigTable`], credentials remain opaque references,
//! and live bytes cross only the secret resolver-to-request boundary.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use sim_config::{ConfigTable, ConfigView};
use sim_kernel::{
    CapabilityName, ContentId, Cx, Datum, Error, Expr, Result as SimResult, Symbol, Value,
};
use sim_lib_net_http::{
    Client, Connector, Header, Method, Policy as HttpPolicy, Request, RequestBody, Url,
};
use sim_lib_search_core::{
    ProviderClaim, SearchError, SearchObservation, SearchPage, SearchQuery, SearchSite,
    SearchWireCodec,
};
use sim_lib_skill::{
    SkillCacheMode, SkillCard, SkillCassetteMode, SkillEventSink, SkillPolicy, SkillRole,
    SkillTransport, skill_specific_call_capability,
};
use sim_lib_web_core::DecodeLimits;
use sim_shape::{AnyShape, ExprKind, ExprKindShape, FieldShape, FieldSpec, ListShape, shape_value};

/// Network-free cookbook descriptors embedded for discovery.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

include!("client.rs");
include!("config.rs");
include!("transport.rs");
include!("skill.rs");

#[cfg(test)]
mod tests;
