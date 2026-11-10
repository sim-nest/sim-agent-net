// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Production caller for one admitted effect.
//!
//! The service lock covers `prepare_effect` and `commit_effect` only. The
//! port runs after that lock is dropped, so another caller can acknowledge
//! stop while an effect has not returned.

use std::sync::{Arc, Mutex};

use sim_lib_journal::JournalBackend;
use sim_work_core::CapabilityGrant;

use crate::{
    CancellationPort, DriveResult, EffectPort, InputBatch, Prepared, ReadinessPort,
    RoadmapRunnerService, ServiceError,
};

/// Admits at most one effect or one pure transition, then runs an admitted effect.
///
/// # Errors
/// Returns the service error from preparation, the port, or commit. A poisoned
/// service lock is an adapter failure.
pub fn conduct_one<B, R, E, C>(
    service: &Arc<Mutex<RoadmapRunnerService<B, R, E, C>>>,
    inputs: &InputBatch,
    grant: &CapabilityGrant,
) -> Result<DriveResult, ServiceError>
where
    B: JournalBackend,
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    let (effects, ticket) = {
        let mut service = service
            .lock()
            .map_err(|_| ServiceError::Adapter("runner service lock poisoned".into()))?;
        let effects = service.effect_port();
        match service.prepare_effect(inputs, grant)? {
            Prepared::Done(result) => return Ok(*result),
            Prepared::Ready(ticket) => (effects, ticket),
        }
    };
    let event = effects.invoke(
        &ticket.identity,
        &ticket.leaf,
        &ticket.transition,
        ticket.product.as_ref(),
    );
    let mut service = service
        .lock()
        .map_err(|_| ServiceError::Adapter("runner service lock poisoned".into()))?;
    service.commit_effect(&ticket, event, inputs)
}
