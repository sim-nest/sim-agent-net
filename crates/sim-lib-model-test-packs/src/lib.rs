//! Frozen, validated model-test task packs.
//!
//! Public packs are rebuilt from pinned Git objects. Private bytes enter only
//! through [`PrivatePackLoader`] and can never be returned by public export.

#![forbid(unsafe_code)]

mod assistance;
mod epoch;
mod manifest;
mod privacy;
mod registry;
mod roadmap;
mod selection;
mod work_unit;

pub use assistance::*;
pub use epoch::*;
pub use manifest::*;
pub use privacy::*;
pub use registry::*;
pub use roadmap::*;
pub use selection::*;
pub use work_unit::*;

#[cfg(test)]
mod tests;
