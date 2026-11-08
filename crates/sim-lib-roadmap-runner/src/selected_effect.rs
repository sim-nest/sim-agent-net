// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One admitted product for a selected plan.
//!
//! The port runs with no borrow of the service. A pure proof does not launch
//! a sandbox. A named world or prove document calls the loaded hook.

use std::sync::Mutex;

use sim_kernel::Symbol;
use sim_lib_exec::{ProcessCancellation, SandboxAttempt, SandboxLauncher};
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{ExecutionEvent, Observation};

use super::{Body, ProductPort, bind};
use crate::{
    DirectionKind, DirectionReply, EffectPort, ExecutionIdentity, InputBatch, LocalRoadmapRequest,
    OpenRequest, ProofDisposition, ServiceError, execute_proof_leaf,
};

impl EffectPort for ProductPort {
    fn invoke(
        &self,
        identity: &ExecutionIdentity,
        phase: &PhaseId,
        _: &sim_roadmap_exec_core::Transition,
        product: Option<&Symbol>,
    ) -> Result<ExecutionEvent, ServiceError> {
        let mut body = self
            .body
            .lock()
            .map_err(|_| ServiceError::Adapter("selected plan body poisoned".into()))?;
        let name = product
            .map(ToString::to_string)
            .ok_or_else(|| ServiceError::Adapter("admitted effect has no product".into()))?;
        match name.as_str() {
            "proof" => prove(&mut body)?,
            "mutation" => bind::mutate(&mut body.bound).map_err(ServiceError::Adapter)?,
            "land" => {
                body.landed = Some(bind::land(&mut body.bound).map_err(ServiceError::Adapter)?);
            }
            "deliver" => deliver(&mut body)?,
            "recover" => {
                let current = body
                    .landed
                    .clone()
                    .ok_or_else(|| ServiceError::Adapter("recovery has no landing".into()))?;
                bind::recover(&mut body.bound, &current).map_err(ServiceError::Adapter)?;
            }
            other => return Err(ServiceError::Adapter(format!("unknown product {other}"))),
        }
        let event = observe(&mut body, identity, phase, &name)?;
        body.events.push(event.clone());
        Ok(event)
    }

    fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> {
        Ok(self
            .body
            .lock()
            .map_err(|_| ServiceError::Adapter("selected plan body poisoned".into()))?
            .events
            .clone())
    }
}

fn prove(body: &mut Body) -> Result<(), ServiceError> {
    let receipt = execute_proof_leaf(
        &body.bound.catalog,
        &body.plan.proof_name,
        &Quiet,
        &ProcessCancellation::default(),
        "selected",
    )
    .map_err(|err| ServiceError::Adapter(err.to_string()))?;
    if receipt.disposition != ProofDisposition::Passed {
        return Err(ServiceError::Adapter(format!(
            "proof {} failed",
            body.plan.proof_name
        )));
    }
    if let Some(document) = body.plan.prove_document.clone() {
        let hook = body
            .prove_hook
            .clone()
            .ok_or_else(|| ServiceError::Adapter("prove product is not loaded".into()))?;
        hook(&document).map_err(ServiceError::Adapter)?;
    }
    body.seal_hex = bind::seal(&body.bound).map_err(ServiceError::Adapter)?;
    Ok(())
}

fn deliver(body: &mut Body) -> Result<(), ServiceError> {
    let _head = bind::delivery_current(&body.bound).map_err(ServiceError::Adapter)?;
    if let Some(document) = body.plan.world_document.clone() {
        let hook = body
            .deliver_hook
            .clone()
            .ok_or_else(|| ServiceError::Adapter("world delivery is not loaded".into()))?;
        hook(&document).map_err(ServiceError::Adapter)?;
    }
    Ok(())
}

fn observe(
    body: &mut Body,
    identity: &ExecutionIdentity,
    phase: &PhaseId,
    product: &str,
) -> Result<ExecutionEvent, ServiceError> {
    body.next_head = body.next_head.saturating_add(1);
    let kind = match product {
        "proof" => "start",
        "mutation" => "mutation-committed",
        "land" => "image-observed",
        "deliver" => "parent-accepted",
        "recover" => "source-deck-current",
        other => return Err(ServiceError::Adapter(format!("no observation for {other}"))),
    };
    let mut observation = Observation::new(Symbol::new(kind), bind::cid_byte(body.next_head));
    if kind == "image-observed" {
        observation.image = Some(body.postimage.clone());
    }
    if kind == "source-deck-current" {
        observation.source_deck = Some(body.source_deck.clone());
    }
    Ok(ExecutionEvent {
        execution: identity.execution.clone(),
        phase: phase.clone(),
        attempt: body.attempt.clone(),
        observation,
    })
}

pub(super) fn direction_inputs(
    body: &Mutex<Body>,
    opened: &OpenRequest,
    request: &LocalRoadmapRequest,
) -> Result<InputBatch, String> {
    match &request.answer {
        None => Ok(InputBatch {
            conduct: true,
            ..InputBatch::default()
        }),
        Some(answer) => {
            let body = super::lock(body)?;
            let expected = body
                .expected
                .clone()
                .ok_or_else(|| "direction has not been published".to_owned())?;
            let revision = if request.named_revision.is_some() {
                bind::cid_byte(9)
            } else {
                expected
            };
            let kind = if request.answer_kind.as_deref() == Some("advice") {
                DirectionKind::Advice
            } else {
                DirectionKind::Decision
            };
            Ok(InputBatch {
                reply: Some(DirectionReply {
                    request: bind::content(body.plan.direction.as_bytes()),
                    execution: opened.authority.identity.execution.clone(),
                    expected_revision: revision,
                    answer_kind: kind,
                    answer: answer.clone(),
                    decision: bind::content(answer.as_bytes()),
                }),
                ..InputBatch::default()
            })
        }
    }
}

struct Quiet;

impl SandboxLauncher for Quiet {
    fn id(&self) -> &str {
        "selected-plan"
    }

    fn launch(&self, _: &sim_lib_exec::SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
        SandboxAttempt::Refused(sim_lib_exec::SandboxRefusal {
            launcher: self.id().into(),
            reason: "pure proof does not launch".into(),
            report: None,
        })
    }
}
