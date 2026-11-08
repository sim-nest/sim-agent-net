// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One-effect conduct for an open execution.
//!
//! Admission records one effect request or one pure transition. The effect
//! runs outside the service. A wake that changes nothing appends no record.
//! Inspection reads the retained result; it does not conduct.

use std::collections::BTreeSet;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Cx, DefaultFactory, Expr, HandleSeed, NoopEvalPolicy, Shape, Symbol};
use sim_roadmap_core::PhaseId;
use sim_roadmap_exec_core::{ExecutionId, Transition};
use sim_shape::ExactExprShape;
use sim_work_core::ResourceNeed;

use sim_lib_journal::{JournalBackend, JournalHead};

use super::{CancellationPort, EffectPort, ReadinessPort, RoadmapRunnerService};
use crate::{ControlIntent, Lease, Mode, WakeCause, WakeEvent};

/// Offered wakes, one optional direction reply, and one optional product step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputBatch {
    /// Wake events offered to this call.
    pub wakes: Vec<WakeEvent>,
    /// Human reply offered to the open direction request.
    pub reply: Option<DirectionReply>,
    /// When set, one admitted leaf effect may run.
    pub conduct: bool,
    /// Opaque product step. The driver does not interpret the symbol.
    pub product: Option<Symbol>,
}

impl InputBatch {
    /// A wake that offers no change.
    pub fn no_change() -> Self {
        Self::default()
    }

    /// One conduct step with no extra wake.
    pub fn conduct() -> Self {
        Self {
            conduct: true,
            ..Self::default()
        }
    }
}

/// Question bound to the execution. The revision fence is added at publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectionQuestion {
    /// Stable request identity. It does not include the revision.
    pub id: ContentId,
    /// Question text.
    pub question: String,
    /// Shape the answer must meet.
    pub answer_kind: DirectionKind,
    /// Authority required before a state change. None keeps the reply informational.
    pub required_authority: Option<Symbol>,
}

/// Fenced direction request published for the current revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectionRequest {
    /// Request identity.
    pub id: ContentId,
    /// Execution the request belongs to.
    pub execution: ExecutionId,
    /// Revision the answer must name.
    pub expected_revision: ContentId,
    /// Question text.
    pub question: String,
    /// Shape the answer must meet.
    pub answer_kind: DirectionKind,
    /// Authority required before a state change.
    pub required_authority: Option<Symbol>,
}

/// Human answer to one published request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectionReply {
    /// Request being answered.
    pub request: ContentId,
    /// Execution the answer names.
    pub execution: ExecutionId,
    /// Revision the answer names.
    pub expected_revision: ContentId,
    /// Shape of the answer.
    pub answer_kind: DirectionKind,
    /// Answer text.
    pub answer: String,
    /// Decision identity. A repeated identity does not authorize a second change.
    pub decision: ContentId,
}

/// Distinct answer shapes. Advice is not a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionKind {
    /// A decision that can change state when authority is present.
    Decision,
    /// Advisory text. It cannot occupy a decision slot.
    Advice,
}

/// Shape of one direction answer.
pub fn direction_shape(kind: DirectionKind) -> ExactExprShape {
    let name = match kind {
        DirectionKind::Advice => "advice",
        DirectionKind::Decision => "decision",
    };
    ExactExprShape::new(Expr::Symbol(Symbol::qualified("direction", name)))
}

/// Provenance reference. It does not satisfy a requirement or grant authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalLineageRef {
    /// Open relation role.
    pub relation: Symbol,
    /// Owner that retains the value.
    pub owner: Symbol,
    /// Shape of the referenced value.
    pub shape: Symbol,
    /// Referenced value.
    pub value: ContentId,
}

/// Selected handoff. Selection and begin authority stay separate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectionHandoff {
    /// Exact source identity.
    pub exact_source: ContentId,
    /// Selected candidate.
    pub candidate: ContentId,
    /// Human selection.
    pub selection: ContentId,
    /// Begin authority. Selection does not fill this field.
    pub begin: Option<ContentId>,
    /// External provenance retained by reference.
    pub external_lineage: Vec<ExternalLineageRef>,
}

/// One projected row inside a retained snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRow {
    /// Row name.
    pub key: String,
    /// Row value.
    pub value: ContentId,
}

/// Revision-fenced projection. Its identity excludes this struct's own id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSnapshot {
    /// Identity derived from the other fields.
    pub id: ContentId,
    /// Execution this projection reads.
    pub execution: ExecutionId,
    /// Revision fence.
    pub revision: ContentId,
    /// Journal head covered by the rows.
    pub journal_head: ContentId,
    /// Rows captured at this revision.
    pub rows: Vec<SnapshotRow>,
}

/// One page of a retained snapshot, or a typed gap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotRead {
    /// Rows from one retained snapshot.
    Page {
        /// Snapshot identity.
        snapshot: ContentId,
        /// Revision fence of that snapshot.
        revision: ContentId,
        /// Journal head covered by the snapshot.
        journal_head: ContentId,
        /// Rows inside the requested bound.
        rows: Vec<SnapshotRow>,
        /// Next offset when more rows remain.
        continuation: Option<u32>,
    },
    /// The requested revision is not the snapshot's revision.
    Stale {
        /// Snapshot that was named.
        snapshot: ContentId,
        /// Revision the snapshot retains.
        retained: ContentId,
        /// Revision the caller named.
        expected: ContentId,
    },
    /// No retained snapshot has that identity.
    Missing,
}

/// Lineage lookup against caller-retained values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineageView {
    /// The referenced value is still present.
    Present(ExternalLineageRef),
    /// The referenced value is absent.
    Gap {
        /// Missing value.
        value: ContentId,
    },
}

/// Result of one conduct call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveResult {
    /// One effect or one pure transition was recorded.
    Advanced {
        /// Revision after the transition.
        transition: ContentId,
        /// Causes that can wake a later call.
        wake: Vec<WakeCause>,
    },
    /// No record was appended.
    Quiescent {
        /// Causes that can wake a later call.
        wake_on: Vec<WakeCause>,
    },
    /// A direction request is published and unanswered.
    NeedDirection {
        /// Published request.
        request: DirectionRequest,
    },
    /// A required resource is not offered.
    Blocked {
        /// Resources still required.
        needs: Vec<ResourceNeed>,
    },
    /// The phase reached a terminal reducer state.
    Complete {
        /// Revision at completion.
        seal_input: ContentId,
    },
}

/// Admission of one effect. The caller invokes the port without borrowing the
/// service, then passes the event to `commit_effect`.
#[derive(Clone, Debug)]
pub struct EffectTicket {
    /// Execution identity the port must observe.
    pub identity: crate::ExecutionIdentity,
    /// Admitted leaf.
    pub leaf: PhaseId,
    /// Reducer state at admission.
    pub transition: Transition,
    /// Journal identity of this admission.
    pub effect_id: String,
    /// Head of the effect-requested record.
    pub intent_head: JournalHead,
    /// Product admitted with this effect. The driver does not interpret it.
    pub product: Option<Symbol>,
}

/// A control step finished, or one effect is admitted and not yet invoked.
#[derive(Clone, Debug)]
pub enum Prepared {
    /// No effect was admitted.
    Done(Box<DriveResult>),
    /// The caller must invoke the port, then commit.
    Ready(Box<EffectTicket>),
}

pub(crate) fn revision_id(opening: &ContentId, n: u64) -> ContentId {
    digest(&[opening.bytes.as_slice(), &n.to_le_bytes()])
}

pub(crate) fn wake_on() -> Vec<WakeCause> {
    vec![
        WakeCause::ResourceOffer,
        WakeCause::Control,
        WakeCause::FactRefresh,
    ]
}

pub(crate) fn digest(parts: &[&[u8]]) -> ContentId {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    let bytes: [u8; 32] = hasher.finalize().into();
    ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), bytes)
}

/// Page key for one verified snapshot. The first page of a causal state uses
/// that state. A later page with a different covered head keeps its own key
/// so the earlier page stays unchanged.
pub(crate) fn snapshot_page_id(
    existing: &[RunSnapshot],
    state: &ContentId,
    covers: &ContentId,
) -> ContentId {
    if existing.iter().any(|item| item.id == *state) {
        digest(&[state.bytes.as_slice(), covers.bytes.as_slice()])
    } else {
        state.clone()
    }
}

pub(crate) fn shapes_admit(expected: DirectionKind, found: DirectionKind) -> bool {
    let mut cx = Cx::new(
        Arc::new(NoopEvalPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x25),
    );
    Shape::is_subshape_of(&direction_shape(found), &mut cx, &direction_shape(expected))
        .ok()
        .flatten()
        == Some(true)
}

pub(crate) fn control_of(inputs: &InputBatch) -> Option<ControlIntent> {
    inputs.wakes.iter().find_map(|wake| match wake {
        WakeEvent::Control(intent) => Some(*intent),
        _ => None,
    })
}

pub(crate) fn offered_resource(
    inputs: &InputBatch,
    needs: &[ResourceNeed],
    seen: &BTreeSet<(String, u64)>,
) -> bool {
    inputs.wakes.iter().any(|wake| match wake {
        WakeEvent::ResourceOffer { kind, amount } if *amount > 0 => {
            needs.iter().any(|need| need.kind.to_string() == *kind)
                && !seen.contains(&(kind.clone(), *amount))
        }
        _ => false,
    })
}

impl<B: JournalBackend, R: ReadinessPort, E: EffectPort, C: CancellationPort>
    RoadmapRunnerService<B, R, E, C>
{
    /// Reads one page of a retained snapshot. Rows never cross revisions.
    pub fn read_snapshot(
        &self,
        snapshot: &ContentId,
        offset: u32,
        limit: u32,
        expected_revision: Option<&ContentId>,
    ) -> SnapshotRead {
        let pages: Vec<_> = self
            .snapshots
            .iter()
            .filter(|item| &item.id == snapshot)
            .collect();
        let Some(first) = pages.first().copied() else {
            return SnapshotRead::Missing;
        };
        let found = if let Some(expected) = expected_revision {
            if let Some(exact) = pages.iter().find(|item| &item.revision == expected) {
                *exact
            } else {
                return SnapshotRead::Stale {
                    snapshot: first.id.clone(),
                    retained: first.revision.clone(),
                    expected: expected.clone(),
                };
            }
        } else {
            first
        };
        let start = offset as usize;
        if start > found.rows.len() || limit == 0 {
            return SnapshotRead::Page {
                snapshot: found.id.clone(),
                revision: found.revision.clone(),
                journal_head: found.journal_head.clone(),
                rows: vec![],
                continuation: None,
            };
        }
        let end = start.saturating_add(limit as usize).min(found.rows.len());
        let continuation = (end < found.rows.len()).then_some(end as u32);
        SnapshotRead::Page {
            snapshot: found.id.clone(),
            revision: found.revision.clone(),
            journal_head: found.journal_head.clone(),
            rows: found.rows[start..end].to_vec(),
            continuation,
        }
    }

    /// Projects lineage against values the caller still holds.
    pub fn lineage(&self, present: &BTreeSet<ContentId>) -> Vec<LineageView> {
        let Some(handoff) = &self.handoff else {
            return vec![];
        };
        handoff
            .external_lineage
            .iter()
            .map(|item| {
                if present.contains(&item.value) {
                    LineageView::Present(item.clone())
                } else {
                    LineageView::Gap {
                        value: item.value.clone(),
                    }
                }
            })
            .collect()
    }

    /// Revision fence of the open execution.
    pub fn revision(&self) -> &ContentId {
        &self.revision
    }

    /// Session mode restored from the journal, or the mode of the last control.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Selected handoff, when one was bound at open.
    pub fn handoff(&self) -> Option<&DirectionHandoff> {
        self.handoff.as_ref()
    }

    /// Leases held by the driver. A resource wait holds none.
    pub fn leases(&self) -> &[Lease] {
        &self.leases
    }

    /// Retained revision-fenced snapshots, oldest first.
    pub fn snapshots(&self) -> &[RunSnapshot] {
        &self.snapshots
    }
}
