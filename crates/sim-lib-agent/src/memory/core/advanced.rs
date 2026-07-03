use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol, Value};
use std::{
    cmp::Ordering,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{FILE_WRITE_CAPABILITY, PersonaMemory, VectorMemory, cosine, embed};

use super::super::store::{
    append_memory_log, flatten_expr_text, load_memory_log, rewrite_memory_log, snapshot_entries,
    value_to_snapshot_expr,
};
use super::types::MemoryBackend;

#[derive(Clone)]
pub(super) struct VectorEntry {
    pub(super) expr: Expr,
    embedding: [f32; crate::embed::EMBED_DIM],
}

#[derive(Clone, Default)]
pub(super) struct PersonaState {
    profile: Vec<(Expr, Expr)>,
    notes: Vec<Expr>,
}

fn lock_vector_entries<'a>(
    entries: &'a Arc<Mutex<Vec<VectorEntry>>>,
) -> Result<std::sync::MutexGuard<'a, Vec<VectorEntry>>> {
    entries
        .lock()
        .map_err(|_| Error::PoisonedLock("vector memory entries"))
}

fn lock_persona_state<'a>(
    state: &'a Arc<Mutex<PersonaState>>,
) -> Result<std::sync::MutexGuard<'a, PersonaState>> {
    state
        .lock()
        .map_err(|_| Error::PoisonedLock("persona memory state"))
}

impl VectorMemory {
    pub(crate) fn open(path: Option<PathBuf>, codecs: Vec<Symbol>) -> Result<Self> {
        let entries = match &path {
            Some(path) => load_memory_log(path)?
                .into_iter()
                .map(VectorEntry::from_expr)
                .collect(),
            None => Vec::new(),
        };
        Ok(Self {
            symbol: Symbol::qualified("memory", "vector"),
            capabilities: path
                .as_ref()
                .map(|_| vec![CapabilityName::new(FILE_WRITE_CAPABILITY)])
                .unwrap_or_default(),
            address: sim_lib_server::ServerAddress::Local,
            codecs,
            path,
            entries: Arc::new(Mutex::new(entries)),
        })
    }
}

impl PersonaMemory {
    pub(crate) fn open(path: Option<PathBuf>, codecs: Vec<Symbol>) -> Result<Self> {
        let state = match &path {
            Some(path) => {
                let entries = load_memory_log(path)?;
                match entries.as_slice() {
                    [] => PersonaState::default(),
                    [snapshot] => persona_state_from_snapshot(snapshot.clone())?,
                    _ => {
                        return Err(Error::HostError(format!(
                            "persona memory {} expected at most one snapshot entry",
                            path.display()
                        )));
                    }
                }
            }
            None => PersonaState::default(),
        };
        Ok(Self {
            symbol: Symbol::qualified("memory", "persona"),
            capabilities: path
                .as_ref()
                .map(|_| vec![CapabilityName::new(FILE_WRITE_CAPABILITY)])
                .unwrap_or_default(),
            address: sim_lib_server::ServerAddress::Local,
            codecs,
            path,
            state: Arc::new(Mutex::new(state)),
        })
    }
}

impl VectorEntry {
    fn from_expr(expr: Expr) -> Self {
        let embedding = embed(&flatten_expr_text(&expr));
        Self { expr, embedding }
    }
}

impl PersonaState {
    fn snapshot(&self) -> Expr {
        Expr::List(vec![Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("state")),
                Expr::Map(self.profile.clone()),
            ),
            (
                Expr::Symbol(Symbol::new("notes")),
                Expr::List(self.notes.clone()),
            ),
        ])])
    }
}

fn persona_state_from_snapshot(snapshot: Expr) -> Result<PersonaState> {
    let (Expr::List(items) | Expr::Vector(items)) = snapshot else {
        return Err(Error::TypeMismatch {
            expected: "persona snapshot list",
            found: "non-list",
        });
    };
    let Some(entry) = items.first() else {
        return Ok(PersonaState::default());
    };
    let Expr::Map(entries) = entry else {
        return Err(Error::TypeMismatch {
            expected: "persona snapshot map",
            found: "non-map",
        });
    };
    let mut profile = Vec::new();
    let mut notes = Vec::new();
    for (key, value) in entries {
        match key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == "state" => {
                let Expr::Map(state_entries) = value else {
                    return Err(Error::TypeMismatch {
                        expected: "persona state map",
                        found: "non-map",
                    });
                };
                profile = state_entries.clone();
            }
            Expr::Symbol(symbol) if symbol.name.as_ref() == "notes" => {
                notes = snapshot_entries(value.clone())?;
            }
            _ => {}
        }
    }
    Ok(PersonaState { profile, notes })
}

impl MemoryBackend for VectorMemory {
    fn append(&self, cx: &mut Cx, msg: Value) -> Result<()> {
        if self.path.is_some() {
            cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
        }
        let expr = value_to_snapshot_expr(cx, msg)?;
        let entry = VectorEntry::from_expr(expr.clone());
        lock_vector_entries(&self.entries)?.push(entry);
        if let Some(path) = &self.path {
            append_memory_log(path, &expr)?;
        }
        Ok(())
    }

    fn recent(&self, cx: &mut Cx, count: u32) -> Result<Vec<Value>> {
        let entries = lock_vector_entries(&self.entries)?;
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        entries[entries.len().saturating_sub(count)..]
            .iter()
            .map(|entry| crate::expr_to_value(cx, &entry.expr))
            .collect()
    }

    fn search(&self, cx: &mut Cx, query: Expr, k: u32) -> Result<Vec<Value>> {
        let query_embedding = embed(&flatten_expr_text(&query));
        let mut scored = lock_vector_entries(&self.entries)?
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                (
                    index,
                    cosine(&query_embedding, &entry.embedding),
                    entry.expr.clone(),
                )
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(Ordering::Equal)
                .then(left.0.cmp(&right.0))
        });
        scored
            .into_iter()
            .take(usize::try_from(k).unwrap_or(usize::MAX))
            .map(|(_, _, expr)| crate::expr_to_value(cx, &expr))
            .collect()
    }

    fn snapshot(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::List(
            lock_vector_entries(&self.entries)?
                .iter()
                .map(|entry| entry.expr.clone())
                .collect(),
        ))
    }

    fn restore(&self, cx: &mut Cx, snap: Expr) -> Result<()> {
        if self.path.is_some() {
            cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
        }
        let entries = snapshot_entries(snap)?
            .into_iter()
            .map(VectorEntry::from_expr)
            .collect::<Vec<_>>();
        if let Some(path) = &self.path {
            let exprs = entries
                .iter()
                .map(|entry| entry.expr.clone())
                .collect::<Vec<_>>();
            rewrite_memory_log(path, &exprs)?;
        }
        *lock_vector_entries(&self.entries)? = entries;
        Ok(())
    }
}

impl MemoryBackend for PersonaMemory {
    fn append(&self, cx: &mut Cx, msg: Value) -> Result<()> {
        if self.path.is_some() {
            cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
        }
        let note = value_to_snapshot_expr(cx, msg)?;
        let mut state = lock_persona_state(&self.state)?;
        state.notes.push(note);
        if let Some(path) = &self.path {
            rewrite_memory_log(path, &[state.snapshot()])?;
        }
        Ok(())
    }

    fn recent(&self, cx: &mut Cx, count: u32) -> Result<Vec<Value>> {
        let state = lock_persona_state(&self.state)?;
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        state.notes[state.notes.len().saturating_sub(count)..]
            .iter()
            .map(|entry| crate::expr_to_value(cx, entry))
            .collect()
    }

    fn search(&self, cx: &mut Cx, query: Expr, k: u32) -> Result<Vec<Value>> {
        let query_terms = flatten_expr_text(&query)
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let state = lock_persona_state(&self.state)?;
        let filtered = state
            .notes
            .iter()
            .rev()
            .filter(|expr| {
                let haystack = flatten_expr_text(expr);
                query_terms.iter().all(|term| haystack.contains(term))
            })
            .take(usize::try_from(k).unwrap_or(usize::MAX))
            .cloned()
            .collect::<Vec<_>>();
        filtered
            .into_iter()
            .rev()
            .map(|expr| crate::expr_to_value(cx, &expr))
            .collect()
    }

    fn snapshot(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(lock_persona_state(&self.state)?.snapshot())
    }

    fn restore(&self, cx: &mut Cx, snap: Expr) -> Result<()> {
        if self.path.is_some() {
            cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
        }
        let state = persona_state_from_snapshot(snap)?;
        if let Some(path) = &self.path {
            rewrite_memory_log(path, &[state.snapshot()])?;
        }
        *lock_persona_state(&self.state)? = state;
        Ok(())
    }
}
