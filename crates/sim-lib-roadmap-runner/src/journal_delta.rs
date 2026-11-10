// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{cell::RefCell, collections::BTreeSet};

use sim_kernel::{ContentId, Datum};
use sim_lib_journal::{JournalBackend, JournalError, JournalHead, JournalObject};

use super::ExecutionJournal;
use crate::causal::{apply_changes, screen_changes};
use crate::journal_share::{self, empty_datum, is_empty};
use crate::{
    DeltaAppend, ExecutionJournalError, ExecutionRecord, MutationError, MutationFence,
    MutationJournal, ObjectKind, ObjectRef, PreparedObject, SemanticChange, VerifiedSnapshot,
    encode_plan, mutation_id_text,
};

impl<B: JournalBackend> ExecutionJournal<B> {
    /// Appends exactly one cause delta. A semantically unchanged revision is a
    /// no-op and preserves the current head.
    pub fn append_delta(
        &self,
        expected: &JournalHead,
        cause: impl Into<String>,
        changes: Vec<SemanticChange>,
        evidence: BTreeSet<ContentId>,
    ) -> Result<DeltaAppend, ExecutionJournalError> {
        let cause = cause.into();
        if cause.is_empty() {
            return Err(ExecutionJournalError::Illegal {
                sequence: expected.sequence + 1,
                reason: "semantic delta cause is empty",
            });
        }
        self.prepare_cache(Some(expected))?;
        let sequence = expected.sequence + 1;
        let (_changed, added, evidence_root) = {
            let guard = self.lock();
            let cache = guard.as_ref().ok_or(ExecutionJournalError::Empty)?;
            if cache.rebuilt.head != *expected {
                return Err(JournalError::WrongHead.into());
            }
            let changed = screen_changes(&cache.rebuilt.causal.facts, &changes, sequence)?;
            let added: Vec<_> = evidence
                .into_iter()
                .filter(|id| !cache.rebuilt.causal.evidence.contains(id))
                .collect();
            if !changed && added.is_empty() {
                return Ok(DeltaAppend::Unchanged(cache.rebuilt.head.clone()));
            }
            (changed, added, cache.rebuilt.evidence_root.clone())
        };
        let share = SharePut::new(
            self,
            ObjectKind::EvidenceSet,
            "persistent evidence set root",
        );
        let new_root = if added.is_empty() {
            evidence_root
        } else {
            journal_share::insert_evidence(&evidence_root, &added, &share)?
        };
        let objects = self.evidence_payload(&new_root, share.last.borrow_mut().take())?;
        let head = self.append(
            Some(expected),
            ExecutionRecord::SemanticDelta {
                cause,
                changes: changes.clone(),
                evidence_set: new_root.clone(),
            },
            objects,
        )?;
        {
            let mut guard = self.lock();
            let Some(cache) = guard.as_mut() else {
                return Ok(DeltaAppend::Appended(head));
            };
            if let Err(error) = apply_changes(&mut cache.rebuilt.causal.facts, &changes, sequence) {
                drop(guard);
                *self.lock() = None;
                return Err(error);
            }
            cache.rebuilt.causal.evidence.extend(added);
            cache.rebuilt.evidence_root = new_root;
        }
        Ok(DeltaAppend::Appended(head))
    }

    fn evidence_payload(
        &self,
        root: &ContentId,
        prepared: Option<PreparedObject>,
    ) -> Result<Vec<PreparedObject>, ExecutionJournalError> {
        if let Some(object) = prepared.filter(|object| &object.reference.content == root) {
            return Ok(vec![object]);
        }
        let datum = if is_empty(root) {
            empty_datum()
        } else {
            self.backend.get_datum(root).map_err(|error| match error {
                JournalError::MissingSemanticObject(_) => ExecutionJournalError::MissingObject,
                other => other.into(),
            })?
        };
        Ok(vec![self.prepare_datum(
            ObjectKind::EvidenceSet,
            datum,
            "persistent evidence set root",
        )?])
    }

    /// Writes a content-addressed snapshot of the verified causal reducer state.
    ///
    /// The snapshot keeps the previous fact tree and stores a new node only for
    /// each changed key. Replay loads that root and applies the later deltas.
    pub fn snapshot(
        &self,
        expected: &JournalHead,
    ) -> Result<VerifiedSnapshot, ExecutionJournalError> {
        self.prepare_cache(Some(expected))?;
        let (facts, facts_root, evidence_set) = {
            let guard = self.lock();
            let cache = guard.as_ref().ok_or(ExecutionJournalError::Empty)?;
            if cache.rebuilt.head != *expected {
                return Err(JournalError::WrongHead.into());
            }
            (
                cache.rebuilt.causal.facts.clone(),
                cache.facts_root.clone(),
                cache.rebuilt.evidence_root.clone(),
            )
        };
        let share = SharePut::new(self, ObjectKind::Snapshot, "shared fact root");
        let facts_root = journal_share::sync_facts(&facts_root, &facts, &share)?;
        let snapshot_object = self.prepare_datum(
            ObjectKind::Snapshot,
            journal_share::snapshot_datum(&facts_root, &evidence_set),
            "verified causal snapshot",
        )?;
        let state_id = snapshot_object.reference.content.clone();
        self.append(
            Some(expected),
            ExecutionRecord::Snapshot {
                covers: expected.entry.clone(),
                state: state_id.clone(),
                evidence_set: evidence_set.clone(),
            },
            vec![snapshot_object],
        )?;
        if let Some(cache) = self.lock().as_mut() {
            cache.facts_root = facts_root;
        }
        Ok(VerifiedSnapshot {
            covers: expected.entry.clone(),
            state: state_id,
            evidence_set,
        })
    }
}

struct SharePut<'a, B: JournalBackend> {
    journal: &'a ExecutionJournal<B>,
    kind: ObjectKind,
    summary: &'static str,
    last: RefCell<Option<PreparedObject>>,
}

impl<'a, B: JournalBackend> SharePut<'a, B> {
    fn new(journal: &'a ExecutionJournal<B>, kind: ObjectKind, summary: &'static str) -> Self {
        Self {
            journal,
            kind,
            summary,
            last: RefCell::new(None),
        }
    }
}

impl<B: JournalBackend> journal_share::ShareIo for SharePut<'_, B> {
    fn load_share(&self, id: &ContentId) -> Result<Datum, ExecutionJournalError> {
        let datum = self
            .journal
            .backend
            .get_datum(id)
            .map_err(|error| match error {
                JournalError::MissingSemanticObject(_) => ExecutionJournalError::MissingObject,
                other => other.into(),
            })?;
        let actual = datum
            .content_id()
            .map_err(|_| ExecutionJournalError::Semantic("shared state"))?;
        if &actual != id {
            return Err(ExecutionJournalError::Semantic("shared state"));
        }
        Ok(datum)
    }

    fn persist_share(&self, datum: Datum) -> Result<ContentId, ExecutionJournalError> {
        // put_datum checks this object only. prepare_datum also rebuilds the
        // whole datum index, which is one full pass per tree node.
        let object = JournalObject::from_datum(datum)?;
        if object.bytes.len() > self.journal.limits.max_object_bytes {
            return Err(ExecutionJournalError::Budget("object"));
        }
        let id = object.id.clone();
        self.journal.backend.put_datum(object.clone())?;
        *self.last.borrow_mut() = Some(PreparedObject {
            reference: ObjectRef {
                kind: self.kind.clone(),
                content: id.clone(),
                bytes: u64::try_from(object.bytes.len()).unwrap_or(u64::MAX),
                summary: crate::journal_util::bounded_summary(self.summary),
            },
            object,
        });
        Ok(id)
    }
}

impl<B: JournalBackend> MutationJournal for ExecutionJournal<B> {
    fn put_plan(&mut self, plan: &crate::SealedMutationPlan) -> Result<(), MutationError> {
        let object = self
            .prepare_object(
                ObjectKind::FileBytes,
                encode_plan(plan),
                "sealed mutation plan",
            )
            .map_err(|error| MutationError::Journal(error.to_string()))?;
        let input = object.reference.clone();
        let state = self
            .rebuild()
            .map_err(|error| MutationError::Journal(error.to_string()))?;
        self.append(
            Some(&state.head),
            ExecutionRecord::EffectRequested {
                effect_id: format!("mutation:{}", mutation_id_text(&plan.id)),
                kind: "sealed-mutation-plan".into(),
                input: Some(input),
            },
            vec![object],
        )
        .map(|_| ())
        .map_err(|error| MutationError::Journal(error.to_string()))
    }

    fn append_fence(
        &mut self,
        plan_id: ContentId,
        fence: MutationFence,
    ) -> Result<(), MutationError> {
        let state = self
            .rebuild()
            .map_err(|error| MutationError::Journal(error.to_string()))?;
        let expected = match fence {
            MutationFence::Prepared => "prepared".into(),
            MutationFence::Applying(index) => format!("applying:{index}"),
            MutationFence::Verifying => "verifying".into(),
            MutationFence::Committed => "committed".into(),
            MutationFence::Ambiguous => "ambiguous".into(),
        };
        self.append(
            Some(&state.head),
            ExecutionRecord::MutationFence {
                mutation_id: mutation_id_text(&plan_id),
                expected,
            },
            Vec::new(),
        )
        .map(|_| ())
        .map_err(|error| MutationError::Journal(error.to_string()))
    }
}
