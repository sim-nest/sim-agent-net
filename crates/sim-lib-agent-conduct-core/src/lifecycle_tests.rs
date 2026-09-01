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
    let replayed =
        DurableAgentRun::receipt_replay(&store, &handle, "graph", Expr::String("result".into()))
            .unwrap();
    assert_eq!(replayed, Expr::String("result".into()));
    assert_eq!(effect_calls, 0);
    effect_calls += 1;
    assert_eq!(effect_calls, 1);
}
