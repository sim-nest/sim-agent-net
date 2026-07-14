//! Remote-backed SIM table directory.
//!
//! This crate exposes a [`RemoteDir`] that projects a remote table site as a
//! SIM table object, along with the [`RemoteDirDescriptor`] citizen record that
//! describes it and the [`RemoteTableSite`] adapter used to back it.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![allow(deprecated)]

pub mod capabilities;
mod citizen;
mod remote_dir;
mod site;

pub use capabilities::table_remote_capability;
pub use citizen::{RemoteDirDescriptor, remote_dir_class_symbol};
pub use remote_dir::{RemoteDir, remote_dir_value};
pub use site::{RemoteTableSite, wrap_remote_table_site};

#[cfg(test)]
mod tests;
