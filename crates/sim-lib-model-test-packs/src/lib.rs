//! Frozen, validated model-test task packs.
//!
//! Public packs are rebuilt from pinned Git objects. Private bytes enter only
//! through [`PrivatePackLoader`] and can never be returned by public export.

#![forbid(unsafe_code)]

mod assistance;
mod domain;
mod epoch;
mod external;
mod generated;
mod manifest;
mod privacy;
mod registry;
mod roadmap;
mod selection;
mod work_unit;

pub use assistance::*;
pub use domain::*;
pub use epoch::*;
pub use external::*;
pub use generated::*;
pub use manifest::*;
pub use privacy::*;
pub use registry::*;
pub use roadmap::*;
pub use selection::*;
pub use work_unit::*;

#[cfg(test)]
mod tests;
