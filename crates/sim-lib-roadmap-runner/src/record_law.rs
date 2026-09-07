use std::collections::BTreeSet;

use crate::{ExecutionJournalError, ExecutionRecord, ObjectRef};

pub(crate) fn record_refs(record: &ExecutionRecord) -> BTreeSet<sim_kernel::ContentId> {
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
        ExecutionRecord::SemanticDelta { evidence_set, .. } => {
            ids.insert(evidence_set.clone());
        }
        ExecutionRecord::Snapshot { state, .. } => {
            ids.insert(state.clone());
        }
        _ => {}
    }
    ids
}
pub(crate) fn validate_next(
    opened: bool,
    records: &[ExecutionRecord],
    record: &ExecutionRecord,
    sequence: u64,
) -> Result<(), ExecutionJournalError> {
    if !opened {
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
    if let ExecutionRecord::SemanticDelta { cause, changes, .. } = record {
        if cause.is_empty() {
            return Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "semantic delta must contain a cause",
            });
        }
        let mut keys = BTreeSet::new();
        if changes.iter().any(|change| !keys.insert(&change.key)) {
            return Err(ExecutionJournalError::Illegal {
                sequence,
                reason: "semantic delta repeats a key",
            });
        }
    }
    Ok(())
}
