#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

use sim_kernel::{ContentId, Datum, Symbol};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

include!("types.rs");
include!("build.rs");
include!("canonical.rs");

#[cfg(test)]
mod tests;
