// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Quiescent roadmap command for a sealed plan with current observations.
//!
//! Each call plans the document, admits the proof receipt and generated
//! postimage into the command cache, conducts to quiescence, and returns
//! the serialized receipt. An expired observation returns a refresh need
//! and does not admit the plan.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use sim_kernel::ContentId;

use super::bind::{self, observe::ObservationState};
use crate::{
    DriveResult, InputBatch, LocalExecutionIdentity, LocalRoadmapReceipt, LocalRoadmapRequest,
    PublicLocalRoadmapPort, conduct_one,
};

pub(super) fn declares_observations(text: &str) -> bool {
    text.lines()
        .any(|line| line.split_whitespace().next() == Some("observation"))
}

/// Admitted sealed plans, keyed by execution identity.
#[derive(Default)]
pub(crate) struct HotCache {
    entries: BTreeMap<String, HotEntry>,
}

struct HotEntry {
    plan_text: String,
    identity: LocalExecutionIdentity,
    postimage: ContentId,
    proof: ContentId,
    generated: ContentId,
    service: Arc<Mutex<super::Service>>,
    body: Arc<Mutex<super::Body>>,
}

pub(super) fn conduct(
    port: &PublicLocalRoadmapPort,
    request: &LocalRoadmapRequest,
    plan_text: &str,
) -> Result<LocalRoadmapReceipt, String> {
    let plan = bind::parse(plan_text)?;
    if plan.observations.is_empty() {
        return Err("selected plan has no observation".into());
    }
    if plan
        .observations
        .iter()
        .any(|(_, state)| *state == ObservationState::Expired)
    {
        return Ok(refresh_receipt(request));
    }
    let bound = bind::bind(&plan)?;
    let postimage = bound.postimage_id.clone();
    let proof = proof_id(&plan);
    let generated = bind::content(&plan.generated);
    let opened = bind::hex(&bound.revision.id);
    let provider_id = bind::hex(&bind::content(plan.provider.as_bytes()));
    let operation_id = bind::hex(&bind::content(plan.operation.as_bytes()));
    drop(bound);

    let key = request.identity.execution.clone();
    let (service, body, cache) = {
        let mut guard = port.hot.lock().map_err(|_| "hot plan cache poisoned")?;
        if let Some(entry) = guard.entries.get(&key) {
            if entry.identity != request.identity {
                return Err("selected plan identity changed".into());
            }
            if entry.plan_text != plan_text
                || entry.postimage != postimage
                || entry.proof != proof
                || entry.generated != generated
            {
                return Err("selected plan changed".into());
            }
            (Arc::clone(&entry.service), Arc::clone(&entry.body), "hit")
        } else {
            let session = super::open_session(port, plan_text, plan, request.identity.clone())?;
            let service = Arc::clone(&session.service);
            let body = Arc::clone(&session.body);
            guard.entries.insert(
                key,
                HotEntry {
                    plan_text: plan_text.to_owned(),
                    identity: request.identity.clone(),
                    postimage,
                    proof,
                    generated,
                    service: Arc::clone(&service),
                    body: Arc::clone(&body),
                },
            );
            (service, body, "admitted")
        }
    };

    let before = super::effect_count(&service);
    let driven = conduct_one(&service, &InputBatch::no_change(), &super::grant())
        .map_err(|err| err.to_string())?;
    if !matches!(driven, DriveResult::Quiescent { .. }) {
        return Err(format!("hot plan was not quiescent: {driven:?}"));
    }
    let effects = super::effect_count(&service);
    if effects != before {
        return Err("hot plan appended an effect".into());
    }
    let seal = super::lock(&body)?.seal_hex.clone();
    let generator = usize::from(!seal.is_empty());
    if generator != 0 || effects != 0 {
        return Err("hot plan started work".into());
    }
    Ok(LocalRoadmapReceipt {
        outcome: "ok".into(),
        identity: request.identity.clone(),
        journal_acknowledged: true,
        detail: format!(
            "opened={opened} seal={seal} provider-id={provider_id} operation-id={operation_id} product=hot outcome=quiescent effects={effects} parsed=1 bound=1 cache={cache} serialized=1 generator={generator} mutation={effects} outward={effects}"
        ),
    })
}

fn refresh_receipt(request: &LocalRoadmapRequest) -> LocalRoadmapReceipt {
    // Parse found an expired observation. Binding, cache admission, and conduct
    // have not run, so the generator, mutation, and outward counts stay at zero.
    LocalRoadmapReceipt {
        outcome: "refresh-need".into(),
        identity: request.identity.clone(),
        journal_acknowledged: false,
        detail: "product=hot outcome=refresh-need effects=0 parsed=1 bound=0 cache=skipped serialized=1 generator=0 mutation=0 outward=0".into(),
    }
}

fn proof_id(plan: &bind::Plan) -> ContentId {
    let mut bytes = plan.proof_name.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend_from_slice(&plan.proof_left);
    bytes.push(0);
    bytes.extend_from_slice(&plan.proof_right);
    bind::content(&bytes)
}

#[cfg(test)]
#[path = "selected_hot_tests.rs"]
mod tests;
