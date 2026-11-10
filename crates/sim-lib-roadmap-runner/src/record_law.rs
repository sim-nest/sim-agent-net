// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

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

/// Law position after the records already accepted.
///
/// Each later record updates this cursor. Receipt and phase checks do not
/// scan the prefix again.
#[derive(Clone, Debug)]
pub(crate) struct LawCursor {
    opened: bool,
    sealed: bool,
    phase: String,
    requested: BTreeSet<String>,
    receipted: BTreeSet<String>,
}

impl LawCursor {
    pub(crate) fn new() -> Self {
        Self {
            opened: false,
            sealed: false,
            phase: "planned".to_owned(),
            requested: BTreeSet::new(),
            receipted: BTreeSet::new(),
        }
    }

    pub(crate) fn advance(
        &mut self,
        record: &ExecutionRecord,
        sequence: u64,
    ) -> Result<(), ExecutionJournalError> {
        if !self.opened {
            return if matches!(record, ExecutionRecord::ExecutionOpened { .. }) {
                self.opened = true;
                Ok(())
            } else {
                Err(illegal(sequence, "genesis must open execution"))
            };
        }
        if matches!(record, ExecutionRecord::ExecutionOpened { .. }) {
            return Err(illegal(sequence, "execution may only open once"));
        }
        if self.sealed {
            return Err(illegal(sequence, "terminal execution is sealed"));
        }
        match record {
            ExecutionRecord::EffectRequested { effect_id, .. } => {
                self.requested.insert(effect_id.clone());
            }
            ExecutionRecord::EffectReceipt { effect_id, .. } => {
                if !self.requested.contains(effect_id) || !self.receipted.insert(effect_id.clone())
                {
                    return Err(illegal(
                        sequence,
                        "receipt must match one unresolved request",
                    ));
                }
            }
            ExecutionRecord::StateTransition { from, to } => {
                if self.phase != *from || !legal_phase(from, to) {
                    return Err(illegal(sequence, "invalid state transition"));
                }
                self.phase.clone_from(to);
            }
            ExecutionRecord::SemanticDelta { cause, changes, .. } => {
                if cause.is_empty() {
                    return Err(illegal(sequence, "semantic delta must contain a cause"));
                }
                let mut keys = BTreeSet::new();
                if changes
                    .iter()
                    .any(|change| !keys.insert(change.key.as_str()))
                {
                    return Err(illegal(sequence, "semantic delta repeats a key"));
                }
            }
            ExecutionRecord::TerminalReceipt { .. } => self.sealed = true,
            _ => {}
        }
        Ok(())
    }
}

fn illegal(sequence: u64, reason: &'static str) -> ExecutionJournalError {
    ExecutionJournalError::Illegal { sequence, reason }
}

fn legal_phase(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("planned", "running")
            | ("running", "reconciling")
            | ("running", "failed")
            | ("reconciling", "succeeded")
            | ("reconciling", "failed")
    )
}

/// A receipt may use the current journal head when the only records after its
/// request are session-mode control and the snapshots that control retains.
pub(crate) fn receipt_parent_is_current(records: &[ExecutionRecord], effect_id: &str) -> bool {
    let mut seen = false;
    for record in records {
        if !seen {
            if matches!(
                record,
                ExecutionRecord::EffectRequested { effect_id: id, .. } if id == effect_id
            ) {
                seen = true;
            }
            continue;
        }
        if !session_control(record) {
            return false;
        }
    }
    seen
}

fn session_control(record: &ExecutionRecord) -> bool {
    match record {
        ExecutionRecord::Snapshot { .. } => true,
        ExecutionRecord::SemanticDelta { changes, .. } => {
            !changes.is_empty() && changes.iter().all(|change| change.key == "session-mode")
        }
        _ => false,
    }
}
