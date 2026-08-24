//! Frozen, validated model-test task packs.
//!
//! Public packs are rebuilt from pinned Git objects. Private bytes enter only
//! through [`PrivatePackLoader`] and can never be returned by public export.

#![forbid(unsafe_code)]

mod epoch;
mod manifest;
mod privacy;
mod registry;
mod selection;

pub use epoch::*;
pub use manifest::*;
pub use privacy::*;
pub use registry::*;
pub use selection::*;

#[cfg(test)]
mod tests;
