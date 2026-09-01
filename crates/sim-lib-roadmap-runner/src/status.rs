// conformance: bounded roadmap status, explanation, redaction, and audit values.

use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::ContentId;
use sim_lib_journal::JournalHead;

use crate::{ExecutionPins, ExecutionRecord, RebuiltExecution};

const MAX_ROWS: usize = 256;
const MAX_TEXT: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MutationStatus {
    Planned,
    ObservedPreimage,
    ObservedPostimage,
    Foreign,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConductTraceRef {
    pub topology: String,
    pub agent: String,
    pub step: String,
    pub evidence: Option<ContentId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionStatus {
    pub execution_id: String,
    pub head: JournalHead,
    pub state: String,
    pub selected_because: String,
    pub pins: ExecutionPins,
    pub last_event: String,
    pub outstanding_effects: Vec<String>,
    pub open_promises: Vec<String>,
    pub ambiguities: Vec<String>,
    pub remaining_record_bytes: usize,
    pub safe_next_actions: Vec<String>,
    pub conduct_trace: Vec<ConductTraceRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusDiff {
    pub from: JournalHead,
    pub to: JournalHead,
    pub added_facts: Vec<String>,
    pub removed_facts: Vec<String>,
}

impl ExecutionStatus {
    /// Pure projection: consumes verified records and has no effect adapter.
    pub fn from_rebuilt(run: &RebuiltExecution, max_execution_bytes: usize) -> Self {
        let mut state = "planned".to_owned();
        let mut requested = BTreeSet::new();
        let mut promises = BTreeSet::new();
        let mut ambiguities = BTreeSet::new();
        for record in &run.records {
            match record {
                ExecutionRecord::StateTransition { to, .. } => state = bounded(to),
                ExecutionRecord::EffectRequested { effect_id, .. } => {
                    requested.insert(bounded(effect_id));
                }
                ExecutionRecord::EffectReceipt { effect_id, .. } => {
                    requested.remove(effect_id);
                }
                ExecutionRecord::Discharge { obligation } => {
                    promises.remove(obligation);
                }
                ExecutionRecord::ProofResult {
                    proof,
                    passed: false,
                    ..
                } => {
                    promises.insert(bounded(proof));
                }
                ExecutionRecord::Ambiguity { reason } => {
                    ambiguities.insert(bounded(reason));
                }
                _ => {}
            }
        }
        let safe_next_actions = if !ambiguities.is_empty() {
            vec![
                "reconcile from verified evidence".into(),
                "do not repeat an uncertain effect".into(),
            ]
        } else if !requested.is_empty() {
            vec!["observe the outstanding effect".into()]
        } else if state == "succeeded" {
            vec!["inspect the terminal receipt".into()]
        } else {
            vec!["advance one admitted effect".into()]
        };
        Self {
            execution_id: bounded(&run.execution_id),
            head: run.head.clone(),
            state,
            selected_because: "compiled roadmap admitted this leaf after its dependencies".into(),
            pins: redacted_pins(&run.pins),
            last_event: run.records.last().map_or_else(|| "none".into(), record_tag),
            outstanding_effects: requested.into_iter().take(MAX_ROWS).collect(),
            open_promises: promises.into_iter().take(MAX_ROWS).collect(),
            ambiguities: ambiguities.into_iter().take(MAX_ROWS).collect(),
            remaining_record_bytes: max_execution_bytes.saturating_sub(run.total_bytes),
            safe_next_actions,
            conduct_trace: Vec::new(),
        }
    }

    pub fn compact(&self) -> String {
        bounded(&format!(
            "{} {} @{}; effects={}; promises={}; ambiguities={}; next={}",
            self.execution_id,
            self.state,
            self.head.sequence,
            self.outstanding_effects.len(),
            self.open_promises.len(),
            self.ambiguities.len(),
            self.safe_next_actions.join(", ")
        ))
    }

    pub fn machine_lines(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("execution".into(), self.execution_id.clone()),
            ("state".into(), self.state.clone()),
            ("head.sequence".into(), self.head.sequence.to_string()),
            ("last_event".into(), self.last_event.clone()),
        ];
        for (i, value) in self.safe_next_actions.iter().take(MAX_ROWS).enumerate() {
            rows.push((format!("next.{i}"), bounded(value)));
        }
        rows
    }

    pub fn diff(&self, other: &Self) -> StatusDiff {
        let a = facts(self);
        let b = facts(other);
        StatusDiff {
            from: self.head.clone(),
            to: other.head.clone(),
            added_facts: b.difference(&a).take(MAX_ROWS).cloned().collect(),
            removed_facts: a.difference(&b).take(MAX_ROWS).cloned().collect(),
        }
    }
}

fn facts(s: &ExecutionStatus) -> BTreeSet<String> {
    let mut m = BTreeMap::new();
    m.insert("state", s.state.clone());
    m.insert("last", s.last_event.clone());
    m.insert("effects", s.outstanding_effects.join(","));
    m.insert("promises", s.open_promises.join(","));
    m.insert("ambiguities", s.ambiguities.join(","));
    m.into_iter().map(|(k, v)| format!("{k}={v}")).collect()
}
fn bounded(value: &str) -> String {
    value
        .chars()
        .take(MAX_TEXT)
        .filter(|c| !c.is_control())
        .collect()
}
fn record_tag(record: &ExecutionRecord) -> String {
    match record {
        ExecutionRecord::ExecutionOpened { .. } => "execution-opened",
        ExecutionRecord::StateTransition { .. } => "state-transition",
        ExecutionRecord::EffectRequested { .. } => "effect-requested",
        ExecutionRecord::EffectReceipt { .. } => "effect-receipt",
        ExecutionRecord::MutationFence { .. } => "mutation-fence",
        ExecutionRecord::ProofResult { .. } => "proof-result",
        ExecutionRecord::Discharge { .. } => "discharge",
        ExecutionRecord::Ambiguity { .. } => "ambiguity",
        ExecutionRecord::TerminalReceipt { .. } => "terminal-receipt",
    }
    .into()
}
fn public(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if value.starts_with('/')
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("api_key")
        || lower.contains("credential")
    {
        "[redacted]".into()
    } else {
        bounded(value)
    }
}
fn redacted_pins(p: &ExecutionPins) -> ExecutionPins {
    ExecutionPins {
        conduct: public(&p.conduct),
        policy: public(&p.policy),
        source_deck: p.source_deck.clone(),
        model_pick: public(&p.model_pick),
        runner_generation: public(&p.runner_generation),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::Symbol;

    fn id(n: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [n; 32])
    }
    fn run(records: Vec<ExecutionRecord>) -> RebuiltExecution {
        RebuiltExecution {
            execution_id: "exec".into(),
            pins: ExecutionPins {
                conduct: "/home/private".into(),
                policy: "token=seeded-secret".into(),
                source_deck: id(1),
                model_pick: "model".into(),
                runner_generation: "runner".into(),
            },
            records,
            head: JournalHead {
                sequence: 2,
                entry: id(2),
            },
            total_bytes: 10,
        }
    }

    #[test]
    fn status_is_deterministic_effect_free_bounded_and_redacted() {
        let rebuilt = run(vec![
            ExecutionRecord::EffectRequested {
                effect_id: "e1".into(),
                kind: "process".into(),
                input: None,
            },
            ExecutionRecord::Ambiguity {
                reason: "foreign tree".into(),
            },
        ]);
        let a = ExecutionStatus::from_rebuilt(&rebuilt, 100);
        let b = ExecutionStatus::from_rebuilt(&rebuilt, 100);
        assert_eq!(a, b);
        assert_eq!(a.pins.conduct, "[redacted]");
        assert_eq!(a.pins.policy, "[redacted]");
        assert!(a.compact().len() <= MAX_TEXT);
        assert!(
            a.safe_next_actions
                .iter()
                .any(|x| x.contains("do not repeat"))
        );
    }

    #[test]
    fn diff_is_stable_and_reports_evidence_state_changes() {
        let a = ExecutionStatus::from_rebuilt(&run(vec![]), 100);
        let b = ExecutionStatus::from_rebuilt(
            &run(vec![ExecutionRecord::ProofResult {
                proof: "tests".into(),
                passed: false,
                evidence: None,
            }]),
            100,
        );
        assert_eq!(a.diff(&b), a.diff(&b));
        assert!(a.diff(&b).added_facts.iter().any(|x| x == "promises=tests"));
    }
}
