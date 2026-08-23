use super::support::{eval_cx, flatten_text};
use crate::{
    FairnessGroup, SyntheticRecord, attribution_card, fairness_summary, minimal_counterfactual,
};
use sim_kernel::{
    BTreeDatumStore, Effect, EventKind, EventLedger, Expr, Ref, Symbol, core_any_ref,
    effect_abort_op_key, effect_ledger::EffectLedger, effect_resume_op_key,
};

#[test]
fn attribution_card_reports_event_and_effect_evidence() {
    let run = Ref::Symbol(Symbol::qualified("fixture", "run"));
    let request = Ref::Symbol(Symbol::qualified("fixture", "request"));
    let trace = Ref::Symbol(Symbol::qualified("fixture", "trace"));
    let effect_ref = Ref::Symbol(Symbol::qualified("fixture", "effect"));
    let result_ref = Ref::Symbol(Symbol::qualified("fixture", "result"));

    let mut events = EventLedger::new();
    events.started(run.clone(), request).unwrap();
    events
        .push(run.clone(), EventKind::Trace(trace.clone()))
        .unwrap();
    events.done(run.clone()).unwrap();

    let mut effects = EffectLedger::with_run(run.clone());
    let mut store = BTreeDatumStore::new();
    let effect = Effect::new(
        sim_kernel::HandleSeed::new(1).sequence().next_handle(),
        Symbol::qualified("fixture", "lookup"),
        trace,
        Ref::Symbol(Symbol::qualified("fixture", "input")),
        core_any_ref(),
        effect_resume_op_key(),
        effect_abort_op_key(),
    )
    .with_id(effect_ref.clone());
    effects.record_requested(&mut store, effect).unwrap();
    effects
        .record_resolved(&mut store, effect_ref, result_ref)
        .unwrap();

    let card = attribution_card(run.clone(), &events, &effects).unwrap();

    assert_eq!(card.run, run);
    assert_eq!(card.event_count, 3);
    assert_eq!(card.effect_count, 1);
    assert_eq!(card.unresolved_effect_count, 0);
    assert_eq!(card.evidence.len(), 4);
    assert!(
        card.evidence
            .iter()
            .any(|row| row.kind == Symbol::new("trace"))
    );

    let mut cx = eval_cx();
    let value = card.as_card_value(&mut cx).unwrap();
    let expr = value.object().as_expr(&mut cx).unwrap();
    let text = flatten_text(&expr);
    assert!(text.contains("fairness-attribution"));
    assert!(text.contains("event-count"));
}

#[test]
fn attribution_card_fails_without_evidence() {
    let err = attribution_card(
        Ref::Symbol(Symbol::qualified("fixture", "run")),
        &EventLedger::new(),
        &EffectLedger::new(),
    )
    .unwrap_err();

    assert!(format!("{err:?}").contains("requires event or effect evidence"));
}

#[test]
fn fairness_summary_uses_four_fifths_metric() {
    let reference = FairnessGroup::new("reference", 8, 10).unwrap();
    let comparison = FairnessGroup::new("comparison", 6, 10).unwrap();

    let summary = fairness_summary(&reference, &comparison).unwrap();

    assert_eq!(summary.reference_rate, 0.8);
    assert_eq!(summary.comparison_rate, 0.6);
    assert!((summary.four_fifths_ratio - 0.75).abs() < 1.0e-12);
    assert!(!summary.passes_four_fifths);
    assert!(flatten_text(&summary.as_expr()).contains("four-fifths-ratio"));
}

#[test]
fn counterfactual_search_is_deterministic() {
    let records = vec![
        SyntheticRecord::new(
            "source",
            "comparison",
            false,
            [("score", 0.4), ("risk", 0.3)],
        )
        .unwrap(),
        SyntheticRecord::new("zeta", "reference", true, [("score", 0.6), ("risk", 0.3)]).unwrap(),
        SyntheticRecord::new("alpha", "reference", true, [("score", 0.6), ("risk", 0.3)]).unwrap(),
        SyntheticRecord::new("far", "reference", true, [("score", 0.9), ("risk", 0.8)]).unwrap(),
    ];

    let counterfactual = minimal_counterfactual(&records, "source", true).unwrap();

    assert_eq!(counterfactual.counterfactual_id, "alpha");
    assert_eq!(counterfactual.distance, 0.19999999999999996);
    assert_eq!(counterfactual.changes.len(), 1);
    assert_eq!(counterfactual.changes[0].feature, "score");
    assert!(matches!(counterfactual.as_expr(), Expr::Map(_)));
}

#[test]
fn fairness_and_counterfactual_inputs_fail_closed() {
    assert!(FairnessGroup::new("bad", 3, 2).is_err());
    assert!(FairnessGroup::new("", 1, 2).is_err());
    assert!(SyntheticRecord::new("bad", "group", true, [("score", f64::NAN)]).is_err());

    let records = vec![
        SyntheticRecord::new("source", "comparison", false, [("score", 0.4)]).unwrap(),
        SyntheticRecord::new("other", "comparison", false, [("score", 0.5)]).unwrap(),
    ];

    assert!(minimal_counterfactual(&records, "missing", true).is_err());
    assert!(minimal_counterfactual(&records, "source", false).is_err());
    assert!(minimal_counterfactual(&records, "source", true).is_err());
}
