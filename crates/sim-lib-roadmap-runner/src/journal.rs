use std::{collections::BTreeSet, sync::Arc};

use sim_kernel::{ContentId, Datum, Symbol};
use sim_lib_journal::{
    Journal, JournalBackend, JournalEntry, JournalError, JournalHead, JournalObject,
    PersistentObjectStore, PersistentSemanticObjects, StoredState,
};

use crate::causal::{apply_changes, causal_state_datum, evidence_set_datum, parse_evidence_set};
use crate::journal_util::{bounded_summary, child_id, contains_secret, store_error};
use crate::record_law::{record_refs, validate_next};
use crate::{
    CausalState, DeltaAppend, ExecutionJournalError, ExecutionPins, ExecutionRecord, Limits,
    MutationError, MutationFence, MutationJournal, ObjectKind, ObjectRef, PreparedObject,
    RebuiltExecution, ReplayFailure, RetentionRoots, SemanticChange, VerifiedSnapshot, codec,
    encode_plan, mutation_id_text,
};

pub struct ExecutionJournal<B> {
    backend: Arc<B>,
    execution_id: String,
    limits: Limits,
}

impl<B: JournalBackend> ExecutionJournal<B> {
    pub fn new(backend: Arc<B>, execution_id: impl Into<String>, limits: Limits) -> Self {
        Self {
            backend,
            execution_id: execution_id.into(),
            limits,
        }
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
        let current = self.rebuild()?;
        if current.head != *expected {
            return Err(JournalError::WrongHead.into());
        }
        let mut prospective = current.causal.clone();
        let changed = apply_changes(&mut prospective.facts, &changes, expected.sequence + 1)?;
        let before_evidence = prospective.evidence.len();
        prospective.evidence.extend(evidence);
        if !changed && prospective.evidence.len() == before_evidence {
            return Ok(DeltaAppend::Unchanged(current.head));
        }
        let evidence_value = evidence_set_datum(&prospective.evidence);
        let evidence_object = self.prepare_datum(
            ObjectKind::EvidenceSet,
            evidence_value,
            "persistent evidence set root",
        )?;
        let evidence_set = evidence_object.reference.content.clone();
        let head = self.append(
            Some(expected),
            ExecutionRecord::SemanticDelta {
                cause,
                changes,
                evidence_set,
            },
            vec![evidence_object],
        )?;
        Ok(DeltaAppend::Appended(head))
    }

    /// Writes a content-addressed snapshot of the verified causal reducer state.
    pub fn snapshot(
        &self,
        expected: &JournalHead,
    ) -> Result<VerifiedSnapshot, ExecutionJournalError> {
        let current = self.rebuild()?;
        if current.head != *expected {
            return Err(JournalError::WrongHead.into());
        }
        let evidence_set = evidence_set_datum(&current.causal.evidence)
            .content_id()
            .map_err(|_| ExecutionJournalError::Semantic("evidence set"))?;
        let snapshot_object = self.prepare_datum(
            ObjectKind::Snapshot,
            causal_state_datum(&current.causal, &evidence_set),
            "verified causal snapshot",
        )?;
        let state = snapshot_object.reference.content.clone();
        self.append(
            Some(expected),
            ExecutionRecord::Snapshot {
                covers: expected.entry.clone(),
                state: state.clone(),
                evidence_set: evidence_set.clone(),
            },
            vec![snapshot_object],
        )?;
        Ok(VerifiedSnapshot {
            covers: expected.entry.clone(),
            state,
            evidence_set,
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
        let current = match self.rebuild() {
            Ok(state) => Some(state),
            Err(ExecutionJournalError::Empty) => None,
            Err(error) => return Err(error),
        };
        let records = current
            .as_ref()
            .map_or(&[][..], |state| state.records.as_slice());
        validate_next(!records.is_empty(), records, &record, records.len() as u64)?;
        let record_object =
            JournalObject::from_datum(codec::encode_datum(&self.execution_id, &record))?;
        if record_object.bytes.len() > self.limits.max_record_bytes {
            return Err(ExecutionJournalError::Budget("record"));
        }
        let before = current.as_ref().map_or(0, |s| s.total_bytes);
        let added = record_object
            .bytes
            .len()
            .checked_add(objects.iter().map(|o| o.object.bytes.len()).sum())
            .ok_or(ExecutionJournalError::Budget("execution"))?;
        if before
            .checked_add(added)
            .filter(|n| *n <= self.limits.max_execution_bytes)
            .is_none()
        {
            return Err(ExecutionJournalError::Budget("execution"));
        }
        let referenced = record_refs(&record);
        let supplied: BTreeSet<_> = objects
            .iter()
            .map(|o| o.reference.content.clone())
            .collect();
        let mut available = supplied.clone();
        if let Some(current) = &current {
            available.extend(current.retention.objects.iter().cloned());
        }
        if !referenced.is_subset(&available) {
            return Err(ExecutionJournalError::MissingObject);
        }
        for object in &objects {
            if !referenced.contains(&object.reference.content)
                || object.reference.content != object.object.id
                || object.reference.bytes != object.object.bytes.len() as u64
            {
                return Err(ExecutionJournalError::MissingObject);
            }
        }
        let sequence = expected.map_or(0, |h| h.sequence + 1);
        let mut payloads = vec![record_object.id.clone()];
        payloads.extend(objects.iter().map(|o| o.object.id.clone()));
        let entry = JournalEntry::new(
            sequence,
            expected.map(|h| h.entry.clone()),
            Symbol::qualified("roadmap-execution", record.tag()),
            payloads,
        );
        let mut admitted = vec![record_object];
        admitted.extend(objects.into_iter().map(|o| o.object));
        let journal = Journal::new(self.backend.clone());
        let lease = journal.acquire_lease()?;
        journal
            .publish(&lease, expected, admitted, vec![entry])
            .map_err(Into::into)
    }

    pub fn rebuild(&self) -> Result<RebuiltExecution, ExecutionJournalError> {
        let state = self.backend.read_state()?;
        let journal = Journal::new(self.backend.clone());
        let verification = journal.verify()?;
        let head = verification.head.ok_or(ExecutionJournalError::Empty)?;
        let mut records = Vec::new();
        let mut pins: Option<ExecutionPins> = None;
        let mut total = 0usize;
        let mut causal = CausalState::default();
        let mut snapshots = Vec::new();
        let mut retention = RetentionRoots::default();
        let mut current_evidence_set = evidence_set_datum(&causal.evidence)
            .content_id()
            .map_err(|_| ExecutionJournalError::Semantic("evidence set"))?;
        for entry in &verification.entries {
            if entry.payloads.is_empty()
                || entry.kind.namespace.as_deref() != Some("roadmap-execution")
            {
                return Err(ExecutionJournalError::Illegal {
                    sequence: entry.sequence,
                    reason: "foreign entry",
                });
            }
            let value = state
                .datums
                .get(&entry.payloads[0])
                .ok_or(ExecutionJournalError::MissingObject)?;
            let bytes = state
                .objects
                .get(&entry.payloads[0])
                .ok_or(ExecutionJournalError::MissingObject)?;
            retention.entries.insert(entry.id.clone());
            retention.objects.extend(entry.payloads.iter().cloned());
            total = total
                .checked_add(bytes.len())
                .ok_or(ExecutionJournalError::Budget("execution"))?;
            let (execution, record) = codec::decode_datum(value)?;
            if execution != self.execution_id {
                return Err(ExecutionJournalError::ExecutionIdentity);
            }
            let refs = record_refs(&record);
            let payloads: BTreeSet<_> = entry.payloads.iter().skip(1).cloned().collect();
            if refs != payloads {
                return Err(ExecutionJournalError::MissingObject);
            }
            for id in &refs {
                total = total
                    .checked_add(
                        state
                            .objects
                            .get(id)
                            .ok_or(ExecutionJournalError::MissingObject)?
                            .len(),
                    )
                    .ok_or(ExecutionJournalError::Budget("execution"))?;
            }
            validate_next(pins.is_some(), &records, &record, entry.sequence)?;
            if let ExecutionRecord::ExecutionOpened { pins: opened, .. } = &record {
                pins = Some(opened.clone());
            }
            match &record {
                ExecutionRecord::SemanticDelta {
                    changes,
                    evidence_set,
                    ..
                } => {
                    let changed = apply_changes(&mut causal.facts, changes, entry.sequence)?;
                    let prior_evidence = causal.evidence.clone();
                    let value = state
                        .datums
                        .get(evidence_set)
                        .ok_or(ExecutionJournalError::MissingObject)?;
                    let evidence = parse_evidence_set(value)?;
                    if !causal.evidence.is_subset(&evidence) {
                        return Err(ExecutionJournalError::Semantic(
                            "evidence set dropped a retained member",
                        ));
                    }
                    if !changed && evidence == prior_evidence {
                        return Err(ExecutionJournalError::Illegal {
                            sequence: entry.sequence,
                            reason: "carry-state semantic delta",
                        });
                    }
                    causal.evidence = evidence;
                    current_evidence_set = evidence_set.clone();
                }
                ExecutionRecord::Snapshot {
                    covers,
                    state: state_id,
                    evidence_set,
                } => {
                    if entry.previous.as_ref() != Some(covers)
                        || evidence_set != &current_evidence_set
                    {
                        return Err(ExecutionJournalError::Semantic("snapshot prefix"));
                    }
                    let value = state
                        .datums
                        .get(state_id)
                        .ok_or(ExecutionJournalError::MissingObject)?;
                    let expected = causal_state_datum(&causal, evidence_set);
                    if value != &expected
                        || expected
                            .content_id()
                            .map_err(|_| ExecutionJournalError::Semantic("snapshot identity"))?
                            != *state_id
                    {
                        return Err(ExecutionJournalError::Semantic("snapshot state"));
                    }
                    snapshots.push(VerifiedSnapshot {
                        covers: covers.clone(),
                        state: state_id.clone(),
                        evidence_set: evidence_set.clone(),
                    });
                }
                _ => {}
            }
            records.push(record);
        }
        if total > self.limits.max_execution_bytes {
            return Err(ExecutionJournalError::Budget("execution"));
        }
        Ok(RebuiltExecution {
            execution_id: self.execution_id.clone(),
            pins: pins.ok_or(ExecutionJournalError::ExecutionIdentity)?,
            records,
            head,
            total_bytes: total,
            causal,
            snapshots,
            retention,
        })
    }

    /// Rebuilds while retaining the last generic head whose complete prefix
    /// verified. This is the recovery cursor reported for torn or corrupt tails.
    pub fn rebuild_report(&self) -> Result<RebuiltExecution, ReplayFailure> {
        let state = self.backend.read_state().map_err(|error| ReplayFailure {
            last_verified_head: None,
            error: Box::new(error.into()),
        })?;
        let mut prefix = StoredState {
            objects: state.objects.clone(),
            datums: state.datums.clone(),
            ..StoredState::default()
        };
        let mut last = None;
        for entry in state.entries.values() {
            prefix.entries.insert(entry.sequence, entry.clone());
            prefix.head = Some(JournalHead {
                sequence: entry.sequence,
                entry: entry.id.clone(),
            });
            if sim_lib_journal::replay(prefix.clone()).is_err() {
                break;
            }
            last = prefix.head.clone();
        }
        self.rebuild().map_err(|error| ReplayFailure {
            last_verified_head: last,
            error: Box::new(error),
        })
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
                effect_id: format!("mutation:{}", mutation_id_text(plan.id)),
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
        plan_id: [u8; 32],
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
                mutation_id: mutation_id_text(plan_id),
                expected,
            },
            Vec::new(),
        )
        .map(|_| ())
        .map_err(|error| MutationError::Journal(error.to_string()))
    }
}
