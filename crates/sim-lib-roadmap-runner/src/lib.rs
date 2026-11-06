// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod accept;
mod binding;
mod budget;
mod causal;
mod codec;
mod compatibility;
mod converge;
mod discharge;
mod fixed;
mod implementer;
mod journal;
mod journal_util;
mod local_command;
mod model;
mod mutation;
mod proof;
mod reconcile;
mod record_law;
mod recovery;
mod refiner;
mod schedule;
mod schedule_session;
mod service;
mod source;
mod status;

pub use accept::*;
pub use binding::*;
pub use budget::*;
pub use compatibility::*;
pub use converge::{
    ClaimState, ConvergeError, Descent, Fence, OpenClaim, OpenRequest as RevisionOpen,
    POSTIMAGE_MODE, Proposal, ProposalBody, Revision, SOURCE_SEAL, Step, image_id,
    open as open_revision, proof_frontier, propose, seal_id,
};
pub use discharge::*;
pub use fixed::{
    FactGenerator, FixedError, FixedSeal, FixedStep, retire_duplicate, seal_fixed_point,
};
pub use implementer::*;
pub use journal::ExecutionJournal;
pub use local_command::*;
pub use model::*;
pub use mutation::*;
pub use proof::*;
pub use reconcile::*;
pub use recovery::*;
pub use refiner::*;
pub use schedule::{
    Continuation, FailureFrontier, ProofOutcome, ProofUnit, ResourceAsk, ScheduleError,
    TypedFailure, failure_frontier, information_value, legal_order, resume_frontier,
};
pub use schedule_session::{
    Admit, ControlAck, ControlEntry, ControlIntent, Lease, Liveness, LogicalTime, Mode,
    ModelWorker, Session, WakeCause, WakeEvent, WorkerView,
};
pub use service::*;
pub use source::*;
pub use status::*;

#[cfg(test)]
mod journal_contract_tests;
#[cfg(test)]
mod tests;
