use std::collections::BTreeSet;

use sim_citizen_derive::Citizen;
use sim_kernel::{CapabilityName, Error, Expr, Result, Symbol};

use crate::AgentUsage;

/// Open outcome value returned by a conduct step.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/Outcome", version = 1)]
pub struct AgentOutcome {
    /// Open outcome symbol.
    pub kind: Symbol,
}
impl Default for AgentOutcome {
    fn default() -> Self {
        Self {
            kind: crate::symbols::outcome::CONTINUE(),
        }
    }
}

/// Typed reason that a run stopped.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/Stop", version = 1)]
pub struct AgentStop {
    /// Open stop-code symbol.
    pub code: Symbol,
    /// Redacted diagnostic data.
    pub detail: Expr,
}
impl Default for AgentStop {
    fn default() -> Self {
        Self {
            code: crate::symbols::stop::COMPLETED(),
            detail: Expr::Nil,
        }
    }
}

/// Open, conduct-specific state entries carried by a run frame.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/RunState", version = 1)]
pub struct AgentRunState {
    #[citizen(with = "state_entries")]
    entries: Vec<(Symbol, Expr)>,
}

impl AgentRunState {
    /// Creates checked state. Standard keys are unqualified; extension keys must be namespaced.
    pub fn new(entries: Vec<(Symbol, Expr)>) -> Result<Self> {
        state_entries::validate(&entries)?;
        Ok(Self { entries })
    }
    /// Creates the five standard state entries.
    pub fn standard() -> Self {
        Self {
            entries: vec![
                (Symbol::new("history"), Expr::List(vec![])),
                (Symbol::new("pending-tools"), Expr::List(vec![])),
                (Symbol::new("plan"), Expr::Nil),
                (Symbol::new("phase"), Expr::Nil),
                (Symbol::new("candidate"), Expr::Nil),
            ],
        }
    }
    /// Returns entries in their canonical order.
    pub fn entries(&self) -> &[(Symbol, Expr)] {
        &self.entries
    }
    /// Inserts or replaces one checked state entry.
    pub fn upsert(&mut self, key: Symbol, value: Expr) -> Result<()> {
        let mut entries = self.entries.clone();
        entries.retain(|(candidate, _)| candidate != &key);
        entries.push((key, value));
        state_entries::validate(&entries)?;
        entries.sort_by_key(|(key, _)| key.to_string());
        self.entries = entries;
        Ok(())
    }
    /// Returns one state entry by exact symbol.
    pub fn get(&self, key: &Symbol) -> Option<&Expr> {
        self.entries
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value))
    }
}
impl Default for AgentRunState {
    fn default() -> Self {
        Self::standard()
    }
}

/// Hash-chain cursor stored in a run frame.
#[derive(Clone, Debug, Default, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/JournalHead", version = 1)]
pub struct AgentJournalHead {
    /// Number of records already committed.
    pub sequence: u64,
    /// Canonical hash of the last record, absent for an empty journal.
    pub hash: Option<String>,
}

/// Complete data packet passed into and returned from each agent step.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/RunFrame", version = 1)]
pub struct AgentRunFrame {
    /// Stable open run identity.
    pub run_id: Symbol,
    /// Original input expression.
    pub input: Expr,
    /// Conduct working value.
    pub working: Expr,
    /// Open lifecycle outcome.
    pub outcome: Symbol,
    /// Typed standard and extension state.
    #[citizen(with = "run_state_field")]
    pub state: AgentRunState,
    /// Exact accumulated usage.
    #[citizen(with = "usage_field")]
    pub usage: AgentUsage,
    /// Journal cursor.
    #[citizen(with = "head_field")]
    pub journal: AgentJournalHead,
}

impl AgentRunFrame {
    /// Constructs the standard initial frame.
    pub fn standard(run_id: Symbol, input: Expr) -> Self {
        Self {
            run_id,
            input,
            working: Expr::Nil,
            outcome: crate::symbols::outcome::CONTINUE(),
            state: AgentRunState::standard(),
            usage: AgentUsage::default(),
            journal: AgentJournalHead::default(),
        }
    }
}
impl Default for AgentRunFrame {
    fn default() -> Self {
        Self::standard(Symbol::qualified("run", "example"), Expr::Nil)
    }
}

/// Redacted event committed to the journal.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/Event", version = 1)]
pub struct AgentEvent {
    /// Open event-kind symbol.
    pub kind: Symbol,
    /// Already-redacted event payload.
    pub redacted: Expr,
}
impl AgentEvent {
    /// Constructs an event from explicitly redacted data.
    pub fn new(kind: Symbol, redacted: Expr) -> Self {
        Self { kind, redacted }
    }
}
impl Default for AgentEvent {
    fn default() -> Self {
        Self::new(crate::symbols::event::STEP_COMPLETED(), Expr::Nil)
    }
}

/// One canonical hash-linked journal record.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/JournalRecord", version = 1)]
pub struct AgentJournalRecord {
    /// Zero-based sequence number.
    pub sequence: u64,
    /// Prior record hash, absent only at sequence zero.
    pub prior_hash: Option<String>,
    /// Fingerprint of the validated conduct graph.
    pub graph_fingerprint: String,
    /// Fingerprint of the component bindings.
    pub binding_fingerprint: String,
    /// Encoded redacted event.
    pub event: Expr,
    /// Encoded run frame.
    pub frame: Expr,
    /// Encoded usage snapshot.
    pub usage: Expr,
    /// Effect receipt expressions.
    pub effect_receipts: Vec<Expr>,
    /// Generic topology continuation expression.
    pub continuation: Expr,
    /// SHA-256 of every preceding field in canonical encoding.
    pub hash: String,
}
impl Default for AgentJournalRecord {
    fn default() -> Self {
        Self {
            sequence: 0,
            prior_hash: None,
            graph_fingerprint: String::new(),
            binding_fingerprint: String::new(),
            event: Expr::Nil,
            frame: Expr::Nil,
            usage: Expr::Nil,
            effect_receipts: vec![],
            continuation: Expr::Nil,
            hash: String::new(),
        }
    }
}

/// Data Card declaring the contract of a registered step target.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/StepCard", version = 1)]
pub struct AgentStepCard {
    /// Open step id.
    pub step_id: Symbol,
    /// Step contract version.
    pub version: u64,
    /// Input Shape expression.
    pub input_shape: Expr,
    /// Output Shape expression.
    pub output_shape: Expr,
    /// Open required role kinds.
    pub roles: Vec<Symbol>,
    /// Required capabilities.
    pub capabilities: Vec<CapabilityName>,
    /// Open possible outcomes.
    pub outcomes: Vec<Symbol>,
    /// Whether this step may request an effect.
    pub may_request_effect: bool,
    /// Open usage dimensions this step may charge.
    pub usage_dimensions: Vec<Symbol>,
    /// Redaction policy symbol.
    pub redaction: Symbol,
    /// Replay policy symbol.
    pub replay: Symbol,
}
impl Default for AgentStepCard {
    fn default() -> Self {
        Self {
            step_id: crate::symbols::step::MODEL_TURN(),
            version: 1,
            input_shape: Expr::Symbol(Symbol::qualified("agent", "RunFrame")),
            output_shape: Expr::Symbol(Symbol::qualified("agent", "RunFrame")),
            roles: vec![crate::symbols::role::RUNNER()],
            capabilities: vec![],
            outcomes: vec![crate::symbols::outcome::CONTINUE()],
            may_request_effect: false,
            usage_dimensions: vec![crate::symbols::usage::MODEL_TURN()],
            redaction: Symbol::qualified("agent.redaction", "standard"),
            replay: Symbol::qualified("agent.replay", "recorded"),
        }
    }
}

/// Pure declaration tying a conduct profile to its cards and accepted frame Shape.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/ConductContract", version = 1)]
pub struct AgentConductContract {
    /// Open conduct profile symbol.
    pub profile: Symbol,
    /// Graph fingerprint expected by the contract.
    pub graph_fingerprint: String,
    /// Encoded step Cards.
    pub step_cards: Vec<Expr>,
    /// Run-frame Shape expression.
    pub frame_shape: Expr,
}
impl Default for AgentConductContract {
    fn default() -> Self {
        Self {
            profile: Symbol::qualified("agent.conduct", "example"),
            graph_fingerprint: "sha256:example".into(),
            step_cards: vec![],
            frame_shape: Expr::Symbol(Symbol::qualified("agent", "RunFrame")),
        }
    }
}

mod state_entries {
    use super::*;
    const STANDARD: [&str; 5] = ["history", "pending-tools", "plan", "phase", "candidate"];
    pub fn encode(value: &[(Symbol, Expr)]) -> Expr {
        Expr::Map(
            value
                .iter()
                .map(|(key, value)| (Expr::Symbol(key.clone()), value.clone()))
                .collect(),
        )
    }
    pub fn decode(expr: &Expr) -> Result<Vec<(Symbol, Expr)>> {
        let Expr::Map(rows) = expr else {
            return Err(Error::Eval("run state must be a map".into()));
        };
        let mut out = Vec::new();
        for (key, value) in rows {
            let Expr::Symbol(key) = key else {
                return Err(Error::Eval("run state keys must be symbols".into()));
            };
            out.push((key.clone(), value.clone()));
        }
        validate(&out)?;
        Ok(out)
    }
    pub fn validate(entries: &[(Symbol, Expr)]) -> Result<()> {
        let mut seen = BTreeSet::new();
        for (key, _) in entries {
            if !seen.insert(key.to_string()) {
                return Err(Error::Eval(format!("duplicate run-state entry {key}")));
            }
            if key.namespace.is_none() && !STANDARD.contains(&key.name.as_ref()) {
                return Err(Error::Eval(format!(
                    "extension run-state key {key} must be namespaced"
                )));
            }
        }
        for required in STANDARD {
            if !entries
                .iter()
                .any(|(key, _)| key.namespace.is_none() && key.name.as_ref() == required)
            {
                return Err(Error::Eval(format!(
                    "run state is missing standard entry {required}"
                )));
            }
        }
        Ok(())
    }
}

mod run_state_field {
    use super::*;
    pub fn encode(value: &AgentRunState) -> Expr {
        state_entries::encode(&value.entries)
    }
    pub fn decode(expr: &Expr) -> Result<AgentRunState> {
        AgentRunState::new(state_entries::decode(expr)?)
    }
}
mod usage_field {
    use super::*;
    pub fn encode(value: &AgentUsage) -> Expr {
        Expr::List(
            value
                .quantities()
                .iter()
                .map(|q| Expr::List(vec![Expr::Symbol(q.unit.clone()), int(q.amount)]))
                .collect(),
        )
    }
    pub fn decode(expr: &Expr) -> Result<AgentUsage> {
        let Expr::List(rows) = expr else {
            return Err(Error::Eval("usage must be a list".into()));
        };
        let mut out = vec![];
        for row in rows {
            let Expr::List(parts) = row else {
                return Err(Error::Eval("usage row must be a list".into()));
            };
            let [Expr::Symbol(unit), Expr::Number(amount)] = parts.as_slice() else {
                return Err(Error::Eval("usage row must be (unit amount)".into()));
            };
            out.push(crate::UsageQuantity {
                unit: unit.clone(),
                amount: amount
                    .canonical
                    .parse()
                    .map_err(|_| Error::Eval("usage amount must be a non-negative u64".into()))?,
            });
        }
        AgentUsage::new(out).map_err(|e| Error::Eval(e.to_string()))
    }
    pub(super) fn int(value: u64) -> Expr {
        Expr::Number(sim_kernel::NumberLiteral {
            domain: Symbol::qualified("citizen", "int"),
            canonical: value.to_string(),
        })
    }
}
mod head_field {
    use super::*;
    pub fn encode(value: &AgentJournalHead) -> Expr {
        Expr::List(vec![
            usage_field::int(value.sequence),
            value
                .hash
                .as_ref()
                .map_or(Expr::Nil, |v| Expr::String(v.clone())),
        ])
    }
    pub fn decode(expr: &Expr) -> Result<AgentJournalHead> {
        let Expr::List(parts) = expr else {
            return Err(Error::Eval("journal head must be a list".into()));
        };
        let [Expr::Number(sequence), hash] = parts.as_slice() else {
            return Err(Error::Eval("journal head must be (sequence hash)".into()));
        };
        let sequence = sequence
            .canonical
            .parse()
            .map_err(|_| Error::Eval("journal sequence must be a u64".into()))?;
        let hash = match hash {
            Expr::Nil => None,
            Expr::String(value) if value.starts_with("sha256:") && value.len() == 71 => {
                Some(value.clone())
            }
            Expr::String(_) => {
                return Err(Error::Eval("journal hash must be canonical sha256".into()));
            }
            _ => return Err(Error::Eval("journal hash must be string or nil".into())),
        };
        Ok(AgentJournalHead { sequence, hash })
    }
}
