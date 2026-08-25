#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod codec;
mod journal;
mod model;
mod source;

pub use journal::ExecutionJournal;
pub use model::*;
pub use source::*;

#[cfg(test)]
mod tests;
