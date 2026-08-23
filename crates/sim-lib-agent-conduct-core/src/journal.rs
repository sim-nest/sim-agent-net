use std::fmt;
use std::sync::Arc;

use sim_kernel::{Cx, DefaultFactory, Expr, NoopEvalPolicy, ObjectCompat};

use crate::{AgentEvent, AgentJournalRecord, AgentRunFrame, AgentUsage, sha256};

/// Pure in-memory builder and verifier for an agent journal chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentJournal {
    graph_fingerprint: String,
    binding_fingerprint: String,
    records: Vec<AgentJournalRecord>,
}

/// Journal chain integrity error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalError {
    /// A record skipped or reused a sequence number.
    Sequence {
        /// Required next sequence.
        expected: u64,
        /// Received sequence.
        actual: u64,
    },
    /// A record's prior hash does not name its predecessor.
    PriorHash {
        /// Sequence carrying the invalid predecessor.
        sequence: u64,
    },
    /// A record hash does not match its canonical fields.
    Hash {
        /// Sequence carrying the invalid hash.
        sequence: u64,
    },
    /// The same sequence carried different canonical content.
    DivergentDuplicate {
        /// Conflicting sequence.
        sequence: u64,
    },
    /// A Citizen could not be encoded.
    Encode(String),
}
impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sequence { expected, actual } => {
                write!(f, "journal sequence gap: expected {expected}, got {actual}")
            }
            Self::PriorHash { sequence } => write!(f, "journal prior hash mismatch at {sequence}"),
            Self::Hash { sequence } => write!(f, "journal record hash mismatch at {sequence}"),
            Self::DivergentDuplicate { sequence } => {
                write!(f, "divergent duplicate journal record at {sequence}")
            }
            Self::Encode(message) => write!(f, "journal encoding failed: {message}"),
        }
    }
}
impl std::error::Error for JournalError {}

impl AgentJournal {
    /// Creates an empty journal bound to graph and component fingerprints.
    pub fn new(
        graph_fingerprint: impl Into<String>,
        binding_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            graph_fingerprint: graph_fingerprint.into(),
            binding_fingerprint: binding_fingerprint.into(),
            records: vec![],
        }
    }
    /// Returns committed records.
    pub fn records(&self) -> &[AgentJournalRecord] {
        &self.records
    }
    /// Appends a canonical record.
    pub fn append(
        &mut self,
        event: AgentEvent,
        frame: AgentRunFrame,
        usage: AgentUsage,
        effect_receipts: Vec<Expr>,
        continuation: Expr,
    ) -> Result<&AgentJournalRecord, JournalError> {
        let sequence = u64::try_from(self.records.len()).map_err(|_| JournalError::Sequence {
            expected: u64::MAX,
            actual: u64::MAX,
        })?;
        let prior_hash = self.records.last().map(|record| record.hash.clone());
        let mut cx = context();
        let mut record = AgentJournalRecord {
            sequence,
            prior_hash,
            graph_fingerprint: self.graph_fingerprint.clone(),
            binding_fingerprint: self.binding_fingerprint.clone(),
            event: event
                .as_expr(&mut cx)
                .map_err(|e| JournalError::Encode(e.to_string()))?,
            frame: frame
                .as_expr(&mut cx)
                .map_err(|e| JournalError::Encode(e.to_string()))?,
            usage: usage
                .as_expr(&mut cx)
                .map_err(|e| JournalError::Encode(e.to_string()))?,
            effect_receipts,
            continuation,
            hash: String::new(),
        };
        record.hash = record_hash(&record);
        self.insert(record)?;
        Ok(self.records.last().expect("append inserted one record"))
    }
    /// Inserts a received record, accepting identical duplicates and rejecting divergence.
    pub fn insert(&mut self, record: AgentJournalRecord) -> Result<(), JournalError> {
        if let Some(existing) = self.records.get(record.sequence as usize) {
            return if existing == &record {
                Ok(())
            } else {
                Err(JournalError::DivergentDuplicate {
                    sequence: record.sequence,
                })
            };
        }
        let expected = u64::try_from(self.records.len()).unwrap_or(u64::MAX);
        if record.sequence != expected {
            return Err(JournalError::Sequence {
                expected,
                actual: record.sequence,
            });
        }
        if record.prior_hash != self.records.last().map(|prior| prior.hash.clone()) {
            return Err(JournalError::PriorHash {
                sequence: record.sequence,
            });
        }
        if record.hash != record_hash(&record) {
            return Err(JournalError::Hash {
                sequence: record.sequence,
            });
        }
        self.records.push(record);
        Ok(())
    }
    /// Verifies the complete sequence, predecessor links, fingerprints, and hashes.
    pub fn verify(&self) -> Result<(), JournalError> {
        let mut checked = Self::new(&self.graph_fingerprint, &self.binding_fingerprint);
        for record in &self.records {
            checked.insert(record.clone())?;
        }
        Ok(())
    }
}

fn context() -> Cx {
    Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory))
}

fn record_hash(record: &AgentJournalRecord) -> String {
    let mut bytes = Vec::new();
    field(&mut bytes, record.prior_hash.as_deref().unwrap_or(""));
    bytes.extend_from_slice(&record.sequence.to_be_bytes());
    field(&mut bytes, &record.graph_fingerprint);
    field(&mut bytes, &record.binding_fingerprint);
    expr(&mut bytes, &record.event);
    expr(&mut bytes, &record.frame);
    expr(&mut bytes, &record.usage);
    bytes.extend_from_slice(&(record.effect_receipts.len() as u64).to_be_bytes());
    for receipt in &record.effect_receipts {
        expr(&mut bytes, receipt);
    }
    expr(&mut bytes, &record.continuation);
    format!("sha256:{}", sha256::digest_hex(&bytes))
}
fn field(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn expr(out: &mut Vec<u8>, value: &Expr) {
    field(out, &canonical_expr(value));
}
fn canonical_expr(value: &Expr) -> String {
    match value {
        Expr::Nil => "n".into(),
        Expr::Bool(v) => format!("b{v}"),
        Expr::Number(v) => format!("d{}:{}", v.domain, v.canonical),
        Expr::Symbol(v) => format!("y{v}"),
        Expr::Local(v) => format!("o{v}"),
        Expr::String(v) => format!("s{}:{v}", v.len()),
        Expr::Bytes(v) => format!("x{}", hex(v)),
        Expr::List(v) => sequence('l', v),
        Expr::Vector(v) => sequence('v', v),
        Expr::Map(v) => {
            let mut rows: Vec<_> = v
                .iter()
                .map(|(k, v)| format!("{}={}", canonical_expr(k), canonical_expr(v)))
                .collect();
            rows.sort();
            format!("m{}:{}", rows.len(), rows.join(""))
        }
        Expr::Set(v) => {
            let mut rows: Vec<_> = v.iter().map(canonical_expr).collect();
            rows.sort();
            format!("e{}:{}", rows.len(), rows.join(""))
        }
        Expr::Call { operator, args } => {
            format!("c{}{}", canonical_expr(operator), sequence('a', args))
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => format!(
            "i{operator}{}{}",
            canonical_expr(left),
            canonical_expr(right)
        ),
        Expr::Prefix { operator, arg } => format!("p{operator}{}", canonical_expr(arg)),
        Expr::Postfix { operator, arg } => format!("f{operator}{}", canonical_expr(arg)),
        Expr::Block(v) => sequence('k', v),
        Expr::Quote { mode, expr } => format!("q{mode:?}{}", canonical_expr(expr)),
        Expr::Annotated { expr, annotations } => {
            let mut rows: Vec<_> = annotations
                .iter()
                .map(|(k, v)| format!("{k}={}", canonical_expr(v)))
                .collect();
            rows.sort();
            format!("a{}{}", canonical_expr(expr), rows.join(""))
        }
        Expr::Extension { tag, payload } => format!("z{tag}{}", canonical_expr(payload)),
    }
}
fn sequence(tag: char, values: &[Expr]) -> String {
    format!(
        "{tag}{}:{}",
        values.len(),
        values.iter().map(canonical_expr).collect::<String>()
    )
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
