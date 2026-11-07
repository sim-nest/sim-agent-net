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
mod land;
mod local_command;
mod model;
mod mutation;
mod packet_work;
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
mod transact;

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
pub use land::{
    LandCache, LandPlan, LandStatus, LandWhy, land_apply, land_plan, land_recover, land_status,
    land_why,
};
pub use local_command::*;
pub use model::*;
pub use mutation::*;
pub use packet_work::{
    CanonicalWork, CapturedWork, InputPort, ModelBody, ModelReturn, ProposalModel, WorkReceipt,
    WorkReceiveError, WorkSource, receive_work,
};
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
pub use transact::{
    ArtifactModel, BoundDisclosure, CleanKind, DisclosureClass, FacetEdit, GitFault, GitModel,
    ModelBatch, ModelStore, ModelTip, RepoOutcome, RepoRequest, Rollback, TransactError,
    rollback_model, transact_facets,
};

#[cfg(test)]
mod journal_contract_tests;
#[cfg(test)]
mod tests;
