// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Same-thread sequencer for tests whose effect returns.
//!
//! Production control goes through `conduct_one`, which drops the service
//! lock before invoke. This helper keeps a local service so returning fakes
//! can keep the old call shape. It is not the independent control path.

use sim_lib_journal::JournalBackend;
use sim_work_core::CapabilityGrant;

use crate::{
    CancellationPort, DriveResult, EffectPort, InputBatch, Prepared, ReadinessPort,
    RoadmapRunnerService, ServiceError,
};

pub(crate) fn advance_one_effect<B, R, E, C>(
    service: &mut RoadmapRunnerService<B, R, E, C>,
    inputs: &InputBatch,
    grant: &CapabilityGrant,
) -> Result<DriveResult, ServiceError>
where
    B: JournalBackend,
    R: ReadinessPort,
    E: EffectPort,
    C: CancellationPort,
{
    let effects = service.effect_port();
    match service.prepare_effect(inputs, grant)? {
        Prepared::Done(result) => Ok(*result),
        Prepared::Ready(ticket) => {
            let event = effects.invoke(
                &ticket.identity,
                &ticket.leaf,
                &ticket.transition,
                ticket.product.as_ref(),
            );
            service.commit_effect(&ticket, event, inputs)
        }
    }
}
