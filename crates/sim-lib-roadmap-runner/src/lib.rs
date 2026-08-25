#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod codec;
mod journal;
mod model;

pub use journal::ExecutionJournal;
pub use model::*;

#[cfg(test)]
mod tests;
