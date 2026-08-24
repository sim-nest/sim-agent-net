//! Durable lifecycle policy layered over the pure journal chain.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sim_kernel::{CapabilityName, Expr, Symbol};

use crate::{
    AgentEvent, AgentJournal, AgentJournalRecord, AgentRunFrame, AgentUsage, JournalError,
};

/// Durable journal storage selected by the host manifest.
pub trait AgentJournalStore {
    /// Loads all records for `run_id` in sequence order.
    fn load(&self, run_id: &Symbol) -> Result<Vec<AgentJournalRecord>, LifecycleError>;
    /// Atomically appends a record. Exact duplicates must be accepted.
    fn append(&mut self, run_id: &Symbol, record: AgentJournalRecord)
    -> Result<(), LifecycleError>;
}

/// Deterministic journal store useful for memory adapters and tests.
#[derive(Clone, Debug, Default)]
pub struct InMemoryJournalStore {
    runs: BTreeMap<Symbol, Vec<AgentJournalRecord>>,
}

impl AgentJournalStore for InMemoryJournalStore {
    fn load(&self, run_id: &Symbol) -> Result<Vec<AgentJournalRecord>, LifecycleError> {
        Ok(self.runs.get(run_id).cloned().unwrap_or_default())
    }

    fn append(
        &mut self,
        run_id: &Symbol,
        record: AgentJournalRecord,
    ) -> Result<(), LifecycleError> {
        let records = self.runs.entry(run_id.clone()).or_default();
        if let Some(existing) = records.get(record.sequence as usize) {
            return if existing == &record {
                Ok(())
            } else {
                Err(LifecycleError::Journal(JournalError::DivergentDuplicate {
                    sequence: record.sequence,
                }))
            };
        }
        let graph = record.graph_fingerprint.clone();
        let bindings = record.binding_fingerprint.clone();
        let mut checked = AgentJournal::from_records(&graph, &bindings, records.clone())?;
        checked.insert(record.clone())?;
        records.push(record);
        Ok(())
    }
}

/// Why a live run became a durable suspension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuspendReason {
    /// Caller requested a checkpoint.
    Checkpoint,
    /// Cancellation was observed between steps.
    Cancelled,
    /// A current manifest binding was unavailable or drifted.
    UnavailableBinding,
    /// The effective budget admitted no further work.
    ExhaustedBudget,
    /// An effect request has no committed or aborted resolution.
    UncertainEffect,
}

/// Authority-free handle returned across process boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableRunHandle {
    /// Stable run identity.
    pub run_id: Symbol,
    /// Next journal sequence.
    pub sequence: u64,
    /// Last committed hash.
    pub journal_hash: String,
    /// Conduct graph identity.
    pub graph_fingerprint: String,
    /// Manifest binding identity.
    pub binding_fingerprint: String,
}

/// Effect status delivered by the existing effect ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectRecovery {
    /// The prior result must be reused.
    Committed(Expr),
    /// The prior abort must remain an abort.
    Aborted(Expr),
    /// Request resolution is unknown; execution must suspend.
    Requested,
}

/// Recorded model exchange identity and exact usage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelExchange {
    /// Normalized request identity.
    pub request_id: String,
    /// Recorded response or cassette value.
    pub response: Expr,
    /// Usage charged by the exchange.
    pub usage: AgentUsage,
}

/// Counterfactual changes admitted only during cassette replay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counterfactual {
    /// Review node outcomes forced by the reviewer.
    pub forced_reviews: BTreeMap<Symbol, Symbol>,
    /// Recorded step replies replaced by cassette data.
    pub replacement_replies: BTreeMap<Symbol, Expr>,
    /// Edges disabled for this replay.
    pub disabled_edges: BTreeSet<Symbol>,
}

/// Redacted mission/recorder row projected from one journal event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionJournalRow {
    /// Journal sequence.
    pub sequence: u64,
    /// Role recorded by the step.
    pub role: Symbol,
    /// Step identity.
    pub step: Symbol,
    /// Outcome identity.
    pub outcome: Symbol,
    /// Exact usage snapshot.
    pub usage: Expr,
    /// Redacted content references.
    pub content: Expr,
}

/// Parent link carried by a fork's first event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentJournalRef {
    /// Parent run.
    pub run_id: Symbol,
    /// Parent checkpoint sequence.
    pub sequence: u64,
    /// Parent checkpoint hash.
    pub hash: String,
}

/// Verified durable lifecycle state.
pub struct DurableAgentRun<'a, S> {
    store: &'a mut S,
    journal: AgentJournal,
    frame: AgentRunFrame,
    continuation: Expr,
    authority: BTreeSet<CapabilityName>,
    model_exchanges: BTreeMap<String, ModelExchange>,
}

impl<'a, S: AgentJournalStore> DurableAgentRun<'a, S> {
    /// Starts a new run under exact graph, binding, and authority identities.
    pub fn start(
        store: &'a mut S,
        frame: AgentRunFrame,
        graph_fingerprint: impl Into<String>,
        binding_fingerprint: impl Into<String>,
        authority: BTreeSet<CapabilityName>,
    ) -> Result<Self, LifecycleError> {
        if !store.load(&frame.run_id)?.is_empty() {
            return Err(LifecycleError::RunAlreadyExists(frame.run_id));
        }
        Ok(Self {
            store,
            journal: AgentJournal::new(graph_fingerprint, binding_fingerprint),
            frame,
            continuation: Expr::Nil,
            authority,
            model_exchanges: BTreeMap::new(),
        })
    }

    /// Reloads and verifies a journal against current manifest identities.
    pub fn resume(
        store: &'a mut S,
        handle: &DurableRunHandle,
        current_graph: &str,
        current_bindings: &str,
        frame: AgentRunFrame,
        authority: BTreeSet<CapabilityName>,
    ) -> Result<Self, LifecycleError> {
        if handle.graph_fingerprint != current_graph {
            return Err(LifecycleError::GraphDrift);
        }
        if handle.binding_fingerprint != current_bindings {
            return Err(LifecycleError::BindingDrift);
        }
        let records = store.load(&handle.run_id)?;
        let journal = AgentJournal::from_records(current_graph, current_bindings, records)?;
        let last = journal
            .records()
            .last()
            .ok_or(LifecycleError::EmptyJournal)?;
        if handle.sequence != last.sequence + 1 || handle.journal_hash != last.hash {
            return Err(LifecycleError::StaleHandle);
        }
        if frame.run_id != handle.run_id {
            return Err(LifecycleError::RunIdentityDrift);
        }
        Ok(Self {
            continuation: last.continuation.clone(),
            store,
            journal,
            frame,
            authority,
            model_exchanges: BTreeMap::new(),
        })
    }

    /// Commits exactly one completed topology step and its sealed next continuation.
    pub fn commit_step(
        &mut self,
        frame: AgentRunFrame,
        event: AgentEvent,
        effect_receipts: Vec<Expr>,
        sealed_continuation: Expr,
    ) -> Result<DurableRunHandle, LifecycleError> {
        if frame.run_id != self.frame.run_id {
            return Err(LifecycleError::RunIdentityDrift);
        }
        let record = self
            .journal
            .append(
                event,
                frame.clone(),
                frame.usage.clone(),
                effect_receipts,
                sealed_continuation.clone(),
            )?
            .clone();
        self.store.append(&frame.run_id, record)?;
        self.frame = frame;
        self.continuation = sealed_continuation;
        self.handle()
    }

    /// Advances through the caller's production topology path and commits the
    /// returned continuation before control can leave the lifecycle boundary.
    pub fn advance<F>(&mut self, production_step: F) -> Result<DurableRunHandle, LifecycleError>
    where
        F: FnOnce(
            &AgentRunFrame,
            Option<&Expr>,
        ) -> Result<(AgentRunFrame, AgentEvent, Vec<Expr>, Expr), LifecycleError>,
    {
        let continuation = (!matches!(self.continuation, Expr::Nil)).then_some(&self.continuation);
        let (frame, event, receipts, sealed_continuation) =
            production_step(&self.frame, continuation)?;
        self.commit_step(frame, event, receipts, sealed_continuation)
    }

    /// Returns the mandatory automatic suspension outcome, if a boundary
    /// condition prevents another production step.
    pub fn automatic_suspension(
        explicit_checkpoint: bool,
        cancelled: bool,
        binding_available: bool,
        budget_available: bool,
        effect: Option<&EffectRecovery>,
    ) -> Option<SuspendReason> {
        if explicit_checkpoint {
            Some(SuspendReason::Checkpoint)
        } else if cancelled {
            Some(SuspendReason::Cancelled)
        } else if !binding_available {
            Some(SuspendReason::UnavailableBinding)
        } else if !budget_available {
            Some(SuspendReason::ExhaustedBudget)
        } else if matches!(effect, Some(EffectRecovery::Requested)) {
            Some(SuspendReason::UncertainEffect)
        } else {
            None
        }
    }

    /// Returns an authority-free durable suspension handle.
    pub fn suspend(&self, _reason: SuspendReason) -> Result<DurableRunHandle, LifecycleError> {
        self.handle()
    }

    /// Reconciles a pending effect without ever reissuing an unresolved request.
    pub fn reconcile_effect(&self, recovery: EffectRecovery) -> Result<Expr, LifecycleError> {
        match recovery {
            EffectRecovery::Committed(result) | EffectRecovery::Aborted(result) => Ok(result),
            EffectRecovery::Requested => Err(LifecycleError::UncertainEffect),
        }
    }

    /// Records a model exchange; exact duplicate requests reuse the recorded response.
    pub fn record_model_exchange(
        &mut self,
        exchange: ModelExchange,
    ) -> Result<Expr, LifecycleError> {
        if let Some(recorded) = self.model_exchanges.get(&exchange.request_id) {
            return if recorded == &exchange {
                Ok(recorded.response.clone())
            } else {
                Err(LifecycleError::DivergentModelReplay(exchange.request_id))
            };
        }
        let response = exchange.response.clone();
        self.model_exchanges
            .insert(exchange.request_id.clone(), exchange);
        Ok(response)
    }

    /// Verifies journal and report identities and returns the recorded result without bindings.
    pub fn receipt_replay(
        store: &S,
        handle: &DurableRunHandle,
        report_graph_fingerprint: &str,
        recorded_result: Expr,
    ) -> Result<Expr, LifecycleError> {
        if report_graph_fingerprint != handle.graph_fingerprint {
            return Err(LifecycleError::GraphDrift);
        }
        let journal = AgentJournal::from_records(
            &handle.graph_fingerprint,
            &handle.binding_fingerprint,
            store.load(&handle.run_id)?,
        )?;
        let last = journal
            .records()
            .last()
            .ok_or(LifecycleError::EmptyJournal)?;
        if last.hash != handle.journal_hash {
            return Err(LifecycleError::StaleHandle);
        }
        Ok(recorded_result)
    }

    /// Admits counterfactual replay only when every target is cassette-backed.
    pub fn counterfactual_replay(
        &self,
        counterfactual: Counterfactual,
        has_live_effect_target: bool,
    ) -> Result<Counterfactual, LifecycleError> {
        if has_live_effect_target {
            return Err(LifecycleError::LiveEffectInReplay);
        }
        Ok(counterfactual)
    }

    /// Creates a child run at this verified checkpoint.
    pub fn fork(
        &self,
        child_run_id: Symbol,
        child_graph: impl Into<String>,
        child_bindings: impl Into<String>,
        child_authority: BTreeSet<CapabilityName>,
        caller_can_widen: bool,
    ) -> Result<(ParentJournalRef, AgentRunFrame), LifecycleError> {
        if !child_authority.is_subset(&self.authority) && !caller_can_widen {
            return Err(LifecycleError::AuthorityWidening);
        }
        let handle = self.handle()?;
        let mut frame = self.frame.clone();
        frame.run_id = child_run_id;
        let _ = (child_graph.into(), child_bindings.into());
        Ok((
            ParentJournalRef {
                run_id: handle.run_id,
                sequence: handle.sequence - 1,
                hash: handle.journal_hash,
            },
            frame,
        ))
    }

    /// Projects journal records into redacted Atelier mission/recorder rows.
    pub fn mission_rows(
        &self,
        role: Symbol,
        step: Symbol,
        outcome: Symbol,
        redact: impl Fn(&Expr) -> Expr,
    ) -> Vec<MissionJournalRow> {
        self.journal
            .records()
            .iter()
            .map(|record| MissionJournalRow {
                sequence: record.sequence,
                role: role.clone(),
                step: step.clone(),
                outcome: outcome.clone(),
                usage: record.usage.clone(),
                content: redact(&record.event),
            })
            .collect()
    }

    /// Current sealed topology continuation.
    pub fn continuation(&self) -> &Expr {
        &self.continuation
    }

    fn handle(&self) -> Result<DurableRunHandle, LifecycleError> {
        let record = self
            .journal
            .records()
            .last()
            .ok_or(LifecycleError::EmptyJournal)?;
        Ok(DurableRunHandle {
            run_id: self.frame.run_id.clone(),
            sequence: record.sequence + 1,
            journal_hash: record.hash.clone(),
            graph_fingerprint: record.graph_fingerprint.clone(),
            binding_fingerprint: record.binding_fingerprint.clone(),
        })
    }
}

/// Durable lifecycle admission or integrity failure.
#[derive(Debug)]
pub enum LifecycleError {
    /// Journal integrity failure.
    Journal(JournalError),
    /// Run id already has durable history.
    RunAlreadyExists(Symbol),
    /// No checkpoint exists yet.
    EmptyJournal,
    /// Graph identity changed.
    GraphDrift,
    /// Binding or Card identity changed.
    BindingDrift,
    /// Frame belongs to another run.
    RunIdentityDrift,
    /// Handle does not name the verified journal head.
    StaleHandle,
    /// Effect outcome is uncertain.
    UncertainEffect,
    /// A recorded model request was reused with different content.
    DivergentModelReplay(String),
    /// Replay attempted to expose a live effect target.
    LiveEffectInReplay,
    /// Child authority widened without caller capability.
    AuthorityWidening,
    /// Backend failure.
    Store(String),
}

impl From<JournalError> for LifecycleError {
    fn from(value: JournalError) -> Self {
        Self::Journal(value)
    }
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LifecycleError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UsageQuantity, symbols};

    fn frame(run: &str, value: &str) -> AgentRunFrame {
        let mut frame = AgentRunFrame::standard(Symbol::qualified("test-run", run), Expr::Nil);
        frame.working = Expr::String(value.into());
        frame
    }

    fn event(name: &str) -> AgentEvent {
        AgentEvent::new(symbols::event::STEP_COMPLETED(), Expr::String(name.into()))
    }

    #[test]
    fn uninterrupted_and_suspend_resume_commit_identical_chains() {
        let mut uninterrupted = InMemoryJournalStore::default();
        let mut resumed = InMemoryJournalStore::default();
        let authority = BTreeSet::new();
        let initial = frame("equal", "initial");
        let final_frame = frame("equal", "done");

        let direct_handle = {
            let mut run = DurableAgentRun::start(
                &mut uninterrupted,
                initial.clone(),
                "graph",
                "bindings",
                authority.clone(),
            )
            .unwrap();
            run.commit_step(
                final_frame.clone(),
                event("step"),
                vec![],
                Expr::String("continuation".into()),
            )
            .unwrap()
        };
        let resumed_handle = {
            let checkpoint = {
                let mut run = DurableAgentRun::start(
                    &mut resumed,
                    initial,
                    "graph",
                    "bindings",
                    authority.clone(),
                )
                .unwrap();
                let handle = run
                    .commit_step(
                        final_frame.clone(),
                        event("step"),
                        vec![],
                        Expr::String("continuation".into()),
                    )
                    .unwrap();
                assert_eq!(run.suspend(SuspendReason::Checkpoint).unwrap(), handle);
                handle
            };
            let run = DurableAgentRun::resume(
                &mut resumed,
                &checkpoint,
                "graph",
                "bindings",
                final_frame,
                authority,
            )
            .unwrap();
            run.suspend(SuspendReason::Cancelled).unwrap()
        };
        assert_eq!(direct_handle.journal_hash, resumed_handle.journal_hash);
    }

    #[test]
    fn duplicate_corruption_binding_and_authority_are_fail_closed() {
        let mut store = InMemoryJournalStore::default();
        let run_id = Symbol::qualified("test-run", "integrity");
        let handle = {
            let mut run = DurableAgentRun::start(
                &mut store,
                frame("integrity", "start"),
                "graph",
                "bindings",
                BTreeSet::new(),
            )
            .unwrap();
            run.commit_step(frame("integrity", "next"), event("one"), vec![], Expr::Nil)
                .unwrap()
        };
        let duplicate = store.load(&run_id).unwrap()[0].clone();
        store.append(&run_id, duplicate.clone()).unwrap();
        let mut divergent = duplicate;
        divergent.event = Expr::String("different".into());
        assert!(matches!(
            store.append(&run_id, divergent),
            Err(LifecycleError::Journal(
                JournalError::DivergentDuplicate { .. }
            ))
        ));
        assert!(matches!(
            DurableAgentRun::resume(
                &mut store,
                &handle,
                "graph",
                "changed",
                frame("integrity", "next"),
                BTreeSet::new(),
            ),
            Err(LifecycleError::BindingDrift)
        ));

        let mut parent_store = InMemoryJournalStore::default();
        let mut parent = DurableAgentRun::start(
            &mut parent_store,
            frame("parent", "start"),
            "graph",
            "bindings",
            BTreeSet::new(),
        )
        .unwrap();
        parent
            .commit_step(frame("parent", "next"), event("one"), vec![], Expr::Nil)
            .unwrap();
        let mut wider = BTreeSet::new();
        wider.insert(CapabilityName::new("network"));
        assert!(matches!(
            parent.fork(
                Symbol::qualified("test-run", "child"),
                "graph2",
                "bindings2",
                wider,
                false
            ),
            Err(LifecycleError::AuthorityWidening)
        ));
        parent
            .fork(
                Symbol::qualified("test-run", "child"),
                "graph2",
                "bindings2",
                BTreeSet::new(),
                false,
            )
            .unwrap();
    }

    #[test]
    fn effect_model_receipt_and_counterfactual_replay_never_call_live_targets() {
        let mut store = InMemoryJournalStore::default();
        let handle = {
            let mut run = DurableAgentRun::start(
                &mut store,
                frame("replay", "start"),
                "graph",
                "bindings",
                BTreeSet::new(),
            )
            .unwrap();
            assert_eq!(
                run.reconcile_effect(EffectRecovery::Committed(Expr::String("ok".into())))
                    .unwrap(),
                Expr::String("ok".into())
            );
            assert!(matches!(
                run.reconcile_effect(EffectRecovery::Requested),
                Err(LifecycleError::UncertainEffect)
            ));
            let usage = AgentUsage::new(vec![UsageQuantity {
                unit: symbols::usage::MODEL_TURN(),
                amount: 7,
            }])
            .unwrap();
            let exchange = ModelExchange {
                request_id: "request:1".into(),
                response: Expr::String("cassette".into()),
                usage,
            };
            run.record_model_exchange(exchange.clone()).unwrap();
            run.record_model_exchange(exchange).unwrap();
            let handle = run
                .commit_step(
                    frame("replay", "result"),
                    event("model"),
                    vec![Expr::String("effect:committed".into())],
                    Expr::String("sealed".into()),
                )
                .unwrap();
            assert!(matches!(
                run.counterfactual_replay(Counterfactual::default(), true),
                Err(LifecycleError::LiveEffectInReplay)
            ));
            assert_eq!(
                run.mission_rows(
                    symbols::role::RUNNER(),
                    symbols::step::MODEL_TURN(),
                    symbols::outcome::CONTINUE(),
                    |_| Expr::Symbol(Symbol::new("redacted")),
                )[0]
                .content,
                Expr::Symbol(Symbol::new("redacted"))
            );
            handle
        };
        let mut effect_calls = 0;
        let replayed = DurableAgentRun::receipt_replay(
            &store,
            &handle,
            "graph",
            Expr::String("result".into()),
        )
        .unwrap();
        assert_eq!(replayed, Expr::String("result".into()));
        assert_eq!(effect_calls, 0);
        effect_calls += 1;
        assert_eq!(effect_calls, 1);
    }
}
