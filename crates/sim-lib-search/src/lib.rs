//! Deterministic federation of provider-neutral search skills.
//!
//! This crate plans first, dispatches within explicit bounds, and records every
//! omission and policy decision. It never owns provider syntax, HTTP policy,
//! rank mathematics, page decoding, or prose answer generation.
#![forbid(unsafe_code)]
mod product;

pub use product::{
    SEARCH_CAPABILITY, SEARCH_VERB, SearchCommandLib, SearchConfig, SearchMode, SearchOperation,
    SearchProduct, SearchProductError, SearchRecord, install_search_skill, search_input_shape,
    search_output_shape,
};

use sim_kernel::{CapabilityName, ContentId, Datum, ShapeRef};
use sim_lib_agent_runner_core::fenced_data_text_for_id;
use sim_lib_rank::{
    EmbeddingIndex, FusionLimits, RankLimits, RankedFusion, RankedList, reciprocal_rank_fusion,
    retrieve_limited,
};
use sim_lib_search_core::{
    AliasEvidence, Citation, ProviderClaim, SearchObservation, SearchPage, SearchQuery,
};
use sim_lib_skill::{SkillCacheMode, SkillCard, SkillRole};
use sim_lib_web_core::{EvidenceSelector, WebRepresentation};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

/// Network-free cookbook descriptors embedded for discovery.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

include!("planning.rs");
include!("dispatch.rs");
include!("fusion.rs");
include!("judge.rs");
include!("research.rs");

#[cfg(test)]
mod tests;
