//! Adapter from the manifest-selected agent memory component to durable journals.

use std::cell::RefCell;

use sim_kernel::{Cx, Expr, Factory, NumberLiteral, Object, Symbol};
use sim_lib_agent_conduct_core::{AgentJournalRecord, AgentJournalStore, LifecycleError};

use crate::MemoryBackend;

/// Journal store backed by an existing agent [`MemoryBackend`].
///
/// Records use a private, versioned data envelope so reopening the adapter does
/// not depend on live Rust object identity. Other memory traffic is ignored.
pub struct MemoryJournalStore<'a> {
    memory: &'a dyn MemoryBackend,
    cx: RefCell<&'a mut Cx>,
}

impl<'a> MemoryJournalStore<'a> {
    /// Wraps the backend selected by the current agent manifest.
    pub fn new(memory: &'a dyn MemoryBackend, cx: &'a mut Cx) -> Self {
        Self {
            memory,
            cx: RefCell::new(cx),
        }
    }
}

impl AgentJournalStore for MemoryJournalStore<'_> {
    fn load(&self, run_id: &Symbol) -> Result<Vec<AgentJournalRecord>, LifecycleError> {
        let mut cx = self.cx.borrow_mut();
        let values = self
            .memory
            .recent(&mut cx, u32::MAX)
            .map_err(|error| LifecycleError::Store(error.to_string()))?;
        let mut records = Vec::new();
        for value in values {
            let expr = value
                .object()
                .as_expr(&mut cx)
                .map_err(|error| LifecycleError::Store(error.to_string()))?;
            if let Some(record) = decode_envelope(&expr, run_id)? {
                records.push(record);
            }
        }
        records.sort_by_key(|record| record.sequence);
        Ok(records)
    }

    fn append(
        &mut self,
        run_id: &Symbol,
        record: AgentJournalRecord,
    ) -> Result<(), LifecycleError> {
        let existing = self.load(run_id)?;
        if let Some(prior) = existing.get(record.sequence as usize) {
            return if prior == &record {
                Ok(())
            } else {
                Err(LifecycleError::Journal(
                    sim_lib_agent_conduct_core::JournalError::DivergentDuplicate {
                        sequence: record.sequence,
                    },
                ))
            };
        }
        let mut cx = self.cx.borrow_mut();
        let value = cx
            .factory()
            .expr(encode_envelope(run_id, &record))
            .map_err(|error| LifecycleError::Store(error.to_string()))?;
        self.memory
            .append(&mut cx, value)
            .map_err(|error| LifecycleError::Store(error.to_string()))
    }
}

fn encode_envelope(run_id: &Symbol, record: &AgentJournalRecord) -> Expr {
    Expr::Map(vec![
        kv(
            "kind",
            Expr::Symbol(Symbol::qualified("agent", "journal-memory-v1")),
        ),
        kv("run-id", Expr::Symbol(run_id.clone())),
        kv("sequence", integer(record.sequence)),
        kv(
            "prior-hash",
            record
                .prior_hash
                .clone()
                .map(Expr::String)
                .unwrap_or(Expr::Nil),
        ),
        kv("graph", Expr::String(record.graph_fingerprint.clone())),
        kv("bindings", Expr::String(record.binding_fingerprint.clone())),
        kv("event", record.event.clone()),
        kv("frame", record.frame.clone()),
        kv("usage", record.usage.clone()),
        kv("effects", Expr::List(record.effect_receipts.clone())),
        kv("continuation", record.continuation.clone()),
        kv("hash", Expr::String(record.hash.clone())),
    ])
}

fn decode_envelope(
    expr: &Expr,
    expected_run: &Symbol,
) -> Result<Option<AgentJournalRecord>, LifecycleError> {
    let Expr::Map(fields) = expr else {
        return Ok(None);
    };
    if get(fields, "kind")
        != Some(&Expr::Symbol(Symbol::qualified(
            "agent",
            "journal-memory-v1",
        )))
    {
        return Ok(None);
    }
    if get(fields, "run-id") != Some(&Expr::Symbol(expected_run.clone())) {
        return Ok(None);
    }
    let sequence = match required(fields, "sequence")? {
        Expr::Number(value) => value
            .canonical
            .parse()
            .map_err(|_| LifecycleError::Store("invalid journal sequence".into()))?,
        _ => return Err(LifecycleError::Store("invalid journal sequence".into())),
    };
    let prior_hash = match required(fields, "prior-hash")? {
        Expr::Nil => None,
        Expr::String(value) => Some(value.clone()),
        _ => return Err(LifecycleError::Store("invalid prior hash".into())),
    };
    Ok(Some(AgentJournalRecord {
        sequence,
        prior_hash,
        graph_fingerprint: string(fields, "graph")?,
        binding_fingerprint: string(fields, "bindings")?,
        event: required(fields, "event")?.clone(),
        frame: required(fields, "frame")?.clone(),
        usage: required(fields, "usage")?.clone(),
        effect_receipts: match required(fields, "effects")? {
            Expr::List(values) => values.clone(),
            _ => return Err(LifecycleError::Store("invalid effect receipts".into())),
        },
        continuation: required(fields, "continuation")?.clone(),
        hash: string(fields, "hash")?,
    }))
}

fn kv(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
fn integer(value: u64) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("citizen", "int"),
        canonical: value.to_string(),
    })
}
fn get<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    fields
        .iter()
        .find_map(|(key, value)| (key == &Expr::Symbol(Symbol::new(name))).then_some(value))
}
fn required<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Result<&'a Expr, LifecycleError> {
    get(fields, name).ok_or_else(|| LifecycleError::Store(format!("missing journal {name}")))
}
fn string(fields: &[(Expr, Expr)], name: &str) -> Result<String, LifecycleError> {
    match required(fields, name)? {
        Expr::String(value) => Ok(value.clone()),
        _ => Err(LifecycleError::Store(format!("invalid journal {name}"))),
    }
}
