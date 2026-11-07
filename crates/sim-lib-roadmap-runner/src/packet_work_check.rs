// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Canonical input checks before any port read.

use super::{
    CanonicalWork, DECLARED_STATUS, Derived, NAMESPACE, SCHEMA, SOURCE_ROLE, TARGET_ROLE,
    WorkReceiveError,
};

pub(super) fn precheck(input: &CanonicalWork) -> Result<Derived, WorkReceiveError> {
    let mirrors_agree = input.mirrors_agree;
    if input.namespace != NAMESPACE {
        return Err(WorkReceiveError::Namespace { mirrors_agree });
    }
    if input.copy.is_empty() || input.copy != input.current {
        return Err(WorkReceiveError::StaleCopy { mirrors_agree });
    }
    if input.schema != SCHEMA || input.phase.is_empty() || input.max_attempts == 0 {
        return Err(WorkReceiveError::Schema { mirrors_agree });
    }
    if input.target_status != DECLARED_STATUS {
        return Err(WorkReceiveError::Status { mirrors_agree });
    }
    if input.source_role != SOURCE_ROLE || input.target_role != TARGET_ROLE {
        return Err(WorkReceiveError::Role { mirrors_agree });
    }
    if input.narrowed_to >= input.unresolved {
        return Err(WorkReceiveError::NoDescent {
            evidence: Vec::new(),
        });
    }
    if input.sources.is_empty() || input.budget_output == 0 {
        return Err(WorkReceiveError::Schema { mirrors_agree });
    }
    let mut bytes = 0u64;
    let mut tokens = 0u64;
    let mut paths = Vec::with_capacity(input.sources.len());
    let mut surfaces = Vec::with_capacity(input.sources.len());
    for source in &input.sources {
        if !valid_name(&source.surface) || !valid_path(&source.path) {
            return Err(WorkReceiveError::Schema { mirrors_agree });
        }
        if surfaces.contains(&source.surface) {
            return Err(WorkReceiveError::Schema { mirrors_agree });
        }
        surfaces.push(source.surface.clone());
        bytes = bytes
            .checked_add(source.body.len() as u64)
            .ok_or(WorkReceiveError::Oversize)?;
        tokens = tokens
            .checked_add(source.body.len() as u64)
            .ok_or(WorkReceiveError::Oversize)?;
        paths.push(source.path.clone());
    }
    let files = u32::try_from(input.sources.len()).map_err(|_| WorkReceiveError::Oversize)?;
    if bytes > input.budget_bytes || files > input.budget_files || tokens > input.budget_tokens {
        return Err(WorkReceiveError::Oversize);
    }
    if !valid_name(&input.target_surface)
        || surfaces.iter().any(|item| item == &input.target_surface)
    {
        return Err(WorkReceiveError::Schema { mirrors_agree });
    }
    let mut resources = input.resources.clone();
    resources.sort_by(|left, right| left.kind.cmp(&right.kind));
    for pair in resources.windows(2) {
        if pair[0].kind == pair[1].kind {
            return Err(WorkReceiveError::Schema { mirrors_agree });
        }
    }
    for resource in &resources {
        if resource.kind == "effect" || resource.kind == "approval" {
            return Err(WorkReceiveError::EffectsOrApproval);
        }
        if !valid_name(&resource.kind) {
            return Err(WorkReceiveError::Schema { mirrors_agree });
        }
    }
    Ok(Derived {
        bytes,
        files,
        tokens,
        paths,
        resources,
        receiver: format!("{NAMESPACE}/{}", input.target_surface),
    })
}

pub(crate) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4_096
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.chars().any(char::is_control)
}

fn valid_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4_096
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && !value.starts_with('/')
        && !value.chars().any(char::is_control)
}
