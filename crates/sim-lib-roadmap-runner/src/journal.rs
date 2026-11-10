// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex, MutexGuard},
};

use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::{
    Journal, JournalBackend, JournalEntry, JournalError, JournalHead, JournalObject,
    PersistentObjectStore, PersistentSemanticObjects,
};

use crate::journal_replay::{self, Fold};
use crate::journal_share::empty_id;
use crate::journal_util::{bounded_summary, child_id, contains_secret, store_error};
use crate::record_law::{LawCursor, record_refs};
use crate::{
    ExecutionJournalError, ExecutionPins, ExecutionRecord, Limits, ObjectKind, ObjectRef,
    PreparedObject, RebuiltExecution, VerifiedSnapshot, codec,
};

#[path = "journal_delta.rs"]
mod delta;

struct Retained {
    rebuilt: RebuiltExecution,
    facts_root: ContentId,
    law: LawCursor,
}

struct Remembered {
    head: JournalHead,
    record: ExecutionRecord,
    entry_id: ContentId,
    payloads: Vec<ContentId>,
    total_bytes: usize,
    law: LawCursor,
}

pub struct ExecutionJournal<B> {
    backend: Arc<B>,
    execution_id: String,
    limits: Limits,
    retained: Mutex<Option<Retained>>,
}

impl<B: JournalBackend> ExecutionJournal<B> {
    pub fn new(backend: Arc<B>, execution_id: impl Into<String>, limits: Limits) -> Self {
        Self {
            backend,
            execution_id: execution_id.into(),
            limits,
            retained: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Retained>> {
        self.retained
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn save(&self, fold: Fold) {
        *self.lock() = Some(Retained {
            facts_root: fold.facts_root,
            rebuilt: fold.rebuilt,
            law: fold.law,
        });
    }

    fn cache_matches(&self, expected: Option<&JournalHead>) -> bool {
        match (self.lock().as_ref(), expected) {
            (None, None) => true,
            (Some(cache), Some(head)) => cache.rebuilt.head == *head,
            _ => false,
        }
    }

    fn prepare_cache(&self, expected: Option<&JournalHead>) -> Result<(), ExecutionJournalError> {
        if self.cache_matches(expected) {
            return Ok(());
        }
        match self.rebuild_cold() {
            Ok(fold) => {
                if expected.is_some_and(|head| fold.rebuilt.head != *head) {
                    return Err(JournalError::WrongHead.into());
                }
                self.save(fold);
                Ok(())
            }
            Err(ExecutionJournalError::Empty) if expected.is_none() => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn rebuild_cold(&self) -> Result<Fold, ExecutionJournalError> {
        let state = self.backend.read_state()?;
        journal_replay::fold(&self.execution_id, &self.limits, &state).map_err(|fault| *fault.error)
    }

    pub fn prepare_object(
        &self,
        kind: ObjectKind,
        bytes: impl Into<Vec<u8>>,
        summary: impl Into<String>,
    ) -> Result<PreparedObject, ExecutionJournalError> {
        let bytes = bytes.into();
        let summary = summary.into();
        if bytes.len() > self.limits.max_object_bytes {
            return Err(ExecutionJournalError::Budget("object"));
        }
        if matches!(kind, ObjectKind::ProcessOutput) && bytes.len() > self.limits.max_stream_bytes {
            return Err(ExecutionJournalError::Budget("stream"));
        }
        if contains_secret(&bytes) || contains_secret(summary.as_bytes()) {
            return Err(ExecutionJournalError::Secret);
        }
        let object = JournalObject::from_bytes(bytes);
        Ok(PreparedObject {
            reference: ObjectRef {
                kind,
                content: object.id.clone(),
                bytes: object.bytes.len() as u64,
                summary: bounded_summary(&summary),
            },
            object,
        })
    }

    /// Persists a semantic Datum through the journal's owned-return object
    /// facet and prepares it as an entry payload.
    pub fn prepare_datum(
        &self,
        kind: ObjectKind,
        value: Datum,
        summary: impl Into<String>,
    ) -> Result<PreparedObject, ExecutionJournalError> {
        let object = JournalObject::from_datum(value.clone())?;
        if object.bytes.len() > self.limits.max_object_bytes {
            return Err(ExecutionJournalError::Budget("object"));
        }
        let mut store = PersistentObjectStore::open(self.backend.clone()).map_err(store_error)?;
        let stored = store.put(value).map_err(store_error)?;
        if stored.meaning != object.id {
            return Err(ExecutionJournalError::MissingObject);
        }
        Ok(PreparedObject {
            reference: ObjectRef {
                kind,
                content: object.id.clone(),
                bytes: object.bytes.len() as u64,
                summary: bounded_summary(&summary.into()),
            },
            object,
        })
    }

    pub fn open(
        &self,
        pins: ExecutionPins,
        expected: Option<&JournalHead>,
    ) -> Result<RebuiltExecution, ExecutionJournalError> {
        match self.rebuild() {
            Err(ExecutionJournalError::Empty) => {
                self.append(
                    expected,
                    ExecutionRecord::ExecutionOpened { pins, parent: None },
                    Vec::new(),
                )?;
                self.rebuild()
            }
            Ok(existing) if existing.pins == pins => Ok(existing),
            Ok(_) => Err(ExecutionJournalError::ChildRequired {
                child_execution_id: child_id(&self.execution_id, &pins),
            }),
            Err(error) => Err(error),
        }
    }

    pub fn append(
        &self,
        expected: Option<&JournalHead>,
        record: ExecutionRecord,
        objects: Vec<PreparedObject>,
    ) -> Result<JournalHead, ExecutionJournalError> {
        self.prepare_cache(expected)?;
        let execution_id = self.execution_id.clone();
        let backend = Arc::clone(&self.backend);
        let max_execution_bytes = self.limits.max_execution_bytes;
        let record_object =
            JournalObject::from_datum(codec::encode_datum(&self.execution_id, &record))?;
        if record_object.bytes.len() > self.limits.max_record_bytes {
            return Err(ExecutionJournalError::Budget("record"));
        }
        let added = record_object
            .bytes
            .len()
            .checked_add(objects.iter().map(|object| object.object.bytes.len()).sum())
            .ok_or(ExecutionJournalError::Budget("execution"))?;
        let referenced = record_refs(&record);
        let supplied: BTreeSet<_> = objects
            .iter()
            .map(|object| object.reference.content.clone())
            .collect();
        for object in &objects {
            if !referenced.contains(&object.reference.content)
                || object.reference.content != object.object.id
                || object.reference.bytes != object.object.bytes.len() as u64
            {
                return Err(ExecutionJournalError::MissingObject);
            }
        }
        let mut guard = self.lock();
        let (before, covered) = match guard.as_ref() {
            Some(cache) => {
                if expected.is_some_and(|head| cache.rebuilt.head != *head) {
                    return Err(JournalError::WrongHead.into());
                }
                (
                    cache.rebuilt.total_bytes,
                    referenced.iter().all(|id| {
                        supplied.contains(id) || cache.rebuilt.retention.objects.contains(id)
                    }),
                )
            }
            None => (0, referenced.iter().all(|id| supplied.contains(id))),
        };
        if !covered {
            return Err(ExecutionJournalError::MissingObject);
        }
        let sequence = guard
            .as_ref()
            .map_or(0, |cache| cache.rebuilt.records.len() as u64);
        let mut law = match guard.as_ref() {
            Some(cache) => cache.law.clone(),
            None => LawCursor::new(),
        };
        law.advance(&record, sequence)?;
        if before
            .checked_add(added)
            .filter(|total| *total <= max_execution_bytes)
            .is_none()
        {
            return Err(ExecutionJournalError::Budget("execution"));
        }
        let mut payloads = vec![record_object.id.clone()];
        payloads.extend(objects.iter().map(|object| object.object.id.clone()));
        let entry = JournalEntry::new(
            sequence,
            expected.map(|head| head.entry.clone()),
            Symbol::qualified("roadmap-execution", record.tag()),
            payloads.clone(),
        );
        let entry_id = entry.id.clone();
        let mut admitted = vec![record_object];
        admitted.extend(objects.into_iter().map(|object| object.object));
        let journal = Journal::new(backend);
        let lease = journal.acquire_lease()?;
        let head = journal.publish(&lease, expected, admitted, vec![entry])?;
        Self::remember(
            &execution_id,
            &mut guard,
            Remembered {
                head: head.clone(),
                record,
                entry_id,
                payloads,
                total_bytes: before + added,
                law,
            },
        );
        Ok(head)
    }

    fn remember(
        execution_id: &str,
        guard: &mut MutexGuard<'_, Option<Retained>>,
        admitted: Remembered,
    ) {
        let Remembered {
            head,
            record,
            entry_id,
            payloads,
            total_bytes,
            law,
        } = admitted;
        let pins = match &record {
            ExecutionRecord::ExecutionOpened { pins, .. } => Some(pins.clone()),
            _ => None,
        };
        let snapshot = match &record {
            ExecutionRecord::Snapshot {
                covers,
                state,
                evidence_set,
            } => Some(VerifiedSnapshot {
                covers: covers.clone(),
                state: state.clone(),
                evidence_set: evidence_set.clone(),
            }),
            _ => None,
        };
        if let Some(cache) = guard.as_mut() {
            if cache.rebuilt.head == head {
                return;
            }
            cache.rebuilt.total_bytes = total_bytes;
            cache.rebuilt.retention.entries.insert(entry_id);
            cache.rebuilt.retention.objects.extend(payloads);
            if let Some(snapshot) = snapshot {
                cache.rebuilt.snapshots.push(snapshot);
            }
            cache.rebuilt.records.push(record);
            cache.rebuilt.head = head;
            cache.law = law;
            return;
        }
        let Some(pins) = pins else {
            return;
        };
        **guard = Some(Retained {
            rebuilt: RebuiltExecution {
                execution_id: execution_id.to_owned(),
                pins,
                records: vec![record],
                opening_head: head.clone(),
                head,
                total_bytes,
                causal: crate::CausalState::default(),
                snapshots: Vec::new(),
                retention: crate::RetentionRoots {
                    entries: BTreeSet::from([entry_id]),
                    objects: payloads.into_iter().collect(),
                },
                causal_applies: 0,
                evidence_root: empty_id(),
            },
            facts_root: empty_id(),
            law,
        });
    }

    /// Reconstructs the execution. A verified snapshot is the causal boundary:
    /// later deltas are applied, and the prefix is not reduced again.
    pub fn rebuild(&self) -> Result<RebuiltExecution, ExecutionJournalError> {
        let fold = self.rebuild_cold()?;
        let rebuilt = fold.rebuilt.clone();
        self.save(fold);
        Ok(rebuilt)
    }

    /// Finds the last verified head in the same pass that rebuilds a valid log.
    ///
    /// A torn or corrupt tail stops at the previous head. A valid log returns
    /// the same execution `rebuild` returns.
    pub fn rebuild_report(&self) -> Result<RebuiltExecution, crate::ReplayFailure> {
        let state = self
            .backend
            .read_state()
            .map_err(|error| crate::ReplayFailure {
                last_verified_head: None,
                error: Box::new(error.into()),
            })?;
        match journal_replay::fold(&self.execution_id, &self.limits, &state) {
            Ok(fold) => {
                let rebuilt = fold.rebuilt.clone();
                self.save(fold);
                Ok(rebuilt)
            }
            Err(fault) => Err(journal_replay::report_failure(fault)),
        }
    }
}
