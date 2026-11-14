// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Observation rows on a sealed selected plan.

use std::collections::{BTreeMap, BTreeSet};

/// Whether one named observation is still current.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationState {
    /// The observation matches the sealed plan.
    Current,
    /// The observation is older than the sealed plan.
    Expired,
}

pub fn observations(
    fields: &BTreeMap<String, Vec<Vec<String>>>,
) -> Result<Vec<(String, ObservationState)>, String> {
    let rows = fields.get("observation").cloned().unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() != 2 {
            return Err("selected plan observation has the wrong shape".into());
        }
        if !seen.insert(row[0].clone()) {
            return Err(format!("selected plan observation {} is repeated", row[0]));
        }
        let state = match row[1].as_str() {
            "current" => ObservationState::Current,
            "expired" => ObservationState::Expired,
            other => {
                return Err(format!(
                    "selected plan observation {} is not current or expired",
                    other
                ));
            }
        };
        out.push((row[0].clone(), state));
    }
    Ok(out)
}
