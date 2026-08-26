#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{Claim, ClaimKind, ContentId, Datum, Ref, Symbol, Visibility};
use sim_source_deck::SourceQuery;

mod id;
pub use id::*;
mod admit;
mod completion;
mod inheritance;
mod path;
mod tree;
pub use admit::{AdmittedPhase, AdmittedRoadmap};
pub use completion::{AggregateAcceptance, ObligationDisposition};
pub use path::CausalPath;

include!("types.rs");
include!("validation.rs");
include!("datum.rs");

#[cfg(test)]
mod tests;
