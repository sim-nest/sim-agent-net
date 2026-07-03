//! HTTP-backed model runners for SIM.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

mod client;
mod redact;
mod runner;
mod stream;

pub use runner::HttpRunner;
