use std::collections::BTreeSet;

// conformance: Forge intent evaluation compiles explicit runtime work.

use sim_kernel::{Symbol, testing::bare_cx as cx};

use crate::{run_eval, standard_eval_arms, standard_eval_corpus};

#[test]
fn report_distinguishes_all_arms() {
    let mut cx = cx();
    let report = run_eval(&mut cx, &standard_eval_corpus(), &standard_eval_arms()).unwrap();
    let names = report
        .arms
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "raw-prose-baseline",
            "compiled-uncached",
            "compiled-cached",
            "compiled-cached-downshifted",
            "identical-request-replay",
        ]
    );
    assert!(
        report.metrics("compiled-uncached").unwrap().tokens
            > report.metrics("compiled-cached").unwrap().tokens
    );
}

#[test]
fn golden_artifact_hit_makes_zero_compiler_calls() {
    let mut cx = cx();
    let corpus = standard_eval_corpus();
    let report = run_eval(&mut cx, &corpus, &standard_eval_arms()).unwrap();
    let cached = report.metrics("compiled-cached").unwrap();

    assert_eq!(cached.compiler_calls, 0);
    assert_eq!(cached.execution_calls, corpus.len() as u64);
    assert_eq!(cached.accuracy, 1.0);
}

#[test]
fn identical_request_replay_makes_zero_execution_calls() {
    let mut cx = cx();
    let corpus = standard_eval_corpus();
    let report = run_eval(&mut cx, &corpus, &standard_eval_arms()).unwrap();
    let replay = report.metrics("identical-request-replay").unwrap();

    assert_eq!(replay.execution_calls, 0);
    assert_eq!(replay.replay_hits, corpus.len() as u64);
    assert_eq!(replay.tokens, 0);
    assert_eq!(replay.accuracy, 1.0);
}

#[test]
fn downshift_arm_holds_accuracy_below_baseline_cost() {
    let mut cx = cx();
    let report = run_eval(&mut cx, &standard_eval_corpus(), &standard_eval_arms()).unwrap();
    let raw = report.metrics("raw-prose-baseline").unwrap();
    let downshift = report.metrics("compiled-cached-downshifted").unwrap();

    assert!(downshift.accuracy >= raw.accuracy);
    assert!(downshift.tokens < raw.tokens);
}

#[test]
fn committed_corpus_covers_the_four_bridge_profiles() {
    let profiles = standard_eval_corpus()
        .into_iter()
        .map(|case| case.profile)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        profiles,
        BTreeSet::from([
            Symbol::qualified("bridge", "ASK"),
            Symbol::qualified("bridge", "BRIEF"),
            Symbol::qualified("bridge", "COLLAB"),
            Symbol::qualified("bridge", "LOOM"),
        ])
    );
}
