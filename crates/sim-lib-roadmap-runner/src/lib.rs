// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod accept;
mod author;
mod binding;
mod budget;
mod causal;
mod codec;
mod compatibility;
mod conduct;
mod converge;
mod discharge;
mod fixed;
mod implementer;
mod journal;
mod journal_replay;
mod journal_share;
mod journal_util;
mod land;
mod local_command;
mod local_emit;
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
mod selected;
mod service;
mod source;
mod status;
mod transact;

pub use accept::*;
pub use author::{
    AuthorError, CompiledAuthoring, Family, FamilyMember, MemberDiff, MemberRow, MemberView,
    Selection, Stance, StatementKind, compile_authoring, diff_members, lint_family, list_family,
    parse_member, select_member, show_member, source_round_trip, statement_shape,
};
pub use binding::*;
pub use budget::*;
pub use compatibility::*;
pub use conduct::conduct_one;
pub use converge::{
    AttachError, AttachStep, ClaimState, ConvergeError, Descent, Fence, OpenClaim,
    OpenRequest as RevisionOpen, POSTIMAGE_MODE, ProofAttachment, Proposal, ProposalBody, Revision,
    SOURCE_SEAL, Step, attach_proofs, image_id, open as open_revision, proof_frontier, propose,
    seal_id,
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
    Admit, ControlAck, ControlEntry, ControlIntent, Lease, Liveness, LogicalTime, MAX_ADMITTED,
    Mode, ModelWorker, Session, WakeCause, WakeEvent, WorkerView,
};
pub use service::*;
pub use sim_lib_exec::{
    ProcessCancellation, SandboxAttempt, SandboxEvidence, SandboxLauncher, SandboxLimits,
    SandboxRefusal, SandboxReport, SandboxRequest, SandboxResult,
};
pub use sim_lib_journal::MemoryBackend;
pub use source::*;
pub use status::*;
pub use transact::{
    ArtifactModel, BoundDisclosure, CleanKind, DisclosureClass, FacetEdit, GitFault, GitModel,
    ModelBatch, ModelStore, ModelTip, RepoOutcome, RepoRequest, Rollback, TransactError,
    rollback_model, transact_facets,
};

#[cfg(test)]
mod drive_control_tests;
#[cfg(test)]
mod drive_fault_tests;
#[cfg(test)]
mod drive_harness;
#[cfg(test)]
mod drive_more_tests;
#[cfg(test)]
mod drive_restore_tests;
#[cfg(test)]
mod drive_tests;
#[cfg(test)]
mod drive_wake_tests;
#[cfg(test)]
mod journal_contract_tests;
#[cfg(test)]
mod journal_replay_tests;
#[cfg(test)]
mod journal_scale_tests;
#[cfg(test)]
mod tests;
