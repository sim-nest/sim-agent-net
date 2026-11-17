// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The parent suite imports `crate::*`. Re-export this manifest's library so
// that suite resolves here instead of in an empty harness crate.
pub use sim_lib_roadmap::*;

#[cfg(test)]
#[path = "../../src/tests.rs"]
mod tests;
