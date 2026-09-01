#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod compile;
mod completion;
mod explain;
mod ground;
mod invalidation;
mod key;
mod projection;
mod promise;
mod query;
mod readiness;

pub use compile::*;
pub use completion::*;
pub use explain::*;
pub use ground::*;
pub use invalidation::*;
pub use key::*;
pub use projection::*;
pub use promise::*;
pub use query::*;
pub use readiness::*;

#[cfg(test)]
mod tests;
