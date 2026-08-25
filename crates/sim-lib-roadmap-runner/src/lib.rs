#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod codec;
mod journal;
mod model;
mod mutation;
mod proof;
mod reconcile;
mod source;

pub use journal::ExecutionJournal;
pub use model::*;
pub use mutation::*;
pub use proof::*;
pub use reconcile::*;
pub use source::*;

#[cfg(test)]
mod tests;
