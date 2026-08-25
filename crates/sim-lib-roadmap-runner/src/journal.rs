use std::{collections::BTreeSet, sync::Arc};

use sha2::{Digest, Sha256};
use sim_kernel::Symbol;
use sim_lib_journal::{
    Journal, JournalBackend, JournalEntry, JournalHead, JournalObject, StoredState,
};

use crate::{
    ExecutionJournalError, ExecutionPins, ExecutionRecord, Limits, ObjectKind, ObjectRef,
    PreparedObject, RebuiltExecution, ReplayFailure, codec,
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
        let current = self.rebuild().ok();
        validate_next(
            current.as_ref(),
            &record,
            current.as_ref().map_or(0, |s| s.records.len() as u64),
        )?;
        let bytes = codec::encode(&self.execution_id, &record);
        if bytes.len() > self.limits.max_record_bytes {
            return Err(ExecutionJournalError::Budget("record"));
        }
        let before = current.as_ref().map_or(0, |s| s.total_bytes);
        let added = bytes
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
        if !referenced.is_subset(&supplied) {
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
        let record_object = JournalObject::from_bytes(bytes);
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
        for entry in &verification.entries {
            if entry.payloads.is_empty()
                || entry.kind.namespace.as_deref() != Some("roadmap-execution")
            {
                return Err(ExecutionJournalError::Illegal {
                    sequence: entry.sequence,
                    reason: "foreign entry",
                });
            }
            let bytes = state
                .objects
                .get(&entry.payloads[0])
                .ok_or(ExecutionJournalError::MissingObject)?;
            total = total
                .checked_add(bytes.len())
                .ok_or(ExecutionJournalError::Budget("execution"))?;
            let (execution, record) = codec::decode(bytes)?;
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
            let partial = pins.as_ref().map(|p| RebuiltExecution {
                execution_id: self.execution_id.clone(),
                pins: p.clone(),
                records: records.clone(),
                head: head.clone(),
                total_bytes: total,
            });
            validate_next(partial.as_ref(), &record, entry.sequence)?;
            if let ExecutionRecord::ExecutionOpened { pins: opened, .. } = &record {
                pins = Some(opened.clone());
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
        })
    }

    /// Rebuilds while retaining the last generic head whose complete prefix
    /// verified. This is the recovery cursor reported for torn or corrupt tails.
    pub fn rebuild_report(&self) -> Result<RebuiltExecution, ReplayFailure> {
        let state = self.backend.read_state().map_err(|error| ReplayFailure {
            last_verified_head: None,
            error: error.into(),
        })?;
        let mut prefix = StoredState {
            objects: state.objects.clone(),
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
            error,
        })
    }
}

fn record_refs(record: &ExecutionRecord) -> BTreeSet<sim_kernel::ContentId> {
    let mut ids = BTreeSet::new();
    let mut add = |v: &Option<ObjectRef>| {
        if let Some(v) = v {
            ids.insert(v.content.clone());
        }
    };
    match record {
        ExecutionRecord::EffectRequested { input, .. } => add(input),
        ExecutionRecord::EffectReceipt { output, .. } => add(output),
        ExecutionRecord::ProofResult { evidence, .. } => add(evidence),
        _ => {}
    }
    ids
}
fn validate_next(
    state: Option<&RebuiltExecution>,
    record: &ExecutionRecord,
    sequence: u64,
) -> Result<(), ExecutionJournalError> {
    if state.is_none() {
        return if matches!(record, ExecutionRecord::ExecutionOpened { .. }) {
            Ok(())
        } else {
            Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "genesis must open execution",
            })
        };
    }
    if matches!(record, ExecutionRecord::ExecutionOpened { .. }) {
        return Err(ExecutionJournalError::Illegal {
            sequence,
            reason: "execution may only open once",
        });
    }
    let records = &state.expect("checked").records;
    if matches!(
        records.last(),
        Some(ExecutionRecord::TerminalReceipt { .. })
    ) {
        return Err(ExecutionJournalError::Illegal {
            sequence,
            reason: "terminal execution is sealed",
        });
    }
    if let ExecutionRecord::EffectReceipt { effect_id, .. } = record {
        let requested = records.iter().any(
            |r| matches!(r,ExecutionRecord::EffectRequested{effect_id:id,..} if id==effect_id),
        );
        let duplicate = records
            .iter()
            .any(|r| matches!(r,ExecutionRecord::EffectReceipt{effect_id:id,..} if id==effect_id));
        if !requested || duplicate {
            return Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "receipt must match one unresolved request",
            });
        }
    }
    if let ExecutionRecord::StateTransition { from, to } = record {
        let current = records
            .iter()
            .rev()
            .find_map(|r| {
                if let ExecutionRecord::StateTransition { to, .. } = r {
                    Some(to.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("planned");
        if current != from
            || !matches!(
                (from.as_str(), to.as_str()),
                ("planned", "running")
                    | ("running", "reconciling")
                    | ("running", "failed")
                    | ("reconciling", "succeeded")
                    | ("reconciling", "failed")
            )
        {
            return Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "invalid state transition",
            });
        }
    }
    Ok(())
}
fn contains_secret(bytes: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    [
        "api_key=",
        "api-key:",
        "authorization: bearer ",
        "password=",
        "private key-----",
        "secret=",
    ]
    .iter()
    .any(|p| lower.contains(p))
}
fn bounded_summary(value: &str) -> String {
    value.chars().take(240).collect()
}
fn child_id(parent: &str, pins: &ExecutionPins) -> String {
    let mut h = Sha256::new();
    h.update(b"sim-roadmap-child-v1\0");
    for s in [
        parent,
        &pins.conduct,
        &pins.policy,
        &pins.model_pick,
        &pins.runner_generation,
        &pins.source_deck.algorithm.as_qualified_str(),
    ] {
        h.update((s.len() as u64).to_be_bytes());
        h.update(s.as_bytes());
    }
    h.update(pins.source_deck.bytes);
    format!("{parent}-child-{:x}", h.finalize())
}
