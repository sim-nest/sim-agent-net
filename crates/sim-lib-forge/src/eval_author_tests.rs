use sim_kernel::testing::bare_cx as cx;

use crate::{AuthorArm, run_author_bench, standard_author_arms, standard_author_cases};

#[test]
fn author_bench_proves_token_and_route_mechanisms() {
    let mut cx = cx();
    let cases = standard_author_cases();
    let report = run_author_bench(&mut cx, &cases, &standard_author_arms()).unwrap();

    let source = report.metrics(AuthorArm::SourcePayload).unwrap();
    let contract = report.metrics(AuthorArm::ContractPayload).unwrap();
    let grammar = report.metrics(AuthorArm::ContractGrammar).unwrap();
    let downshifted = report.metrics(AuthorArm::Downshifted).unwrap();

    assert_eq!(source.accepted_cases, cases.len() as u64);
    assert_eq!(contract.accepted_cases, cases.len() as u64);
    assert_eq!(grammar.accepted_cases, cases.len() as u64);
    assert_eq!(downshifted.accepted_cases, cases.len() as u64);
    assert!(contract.payload_tokens.saturating_mul(100) <= source.payload_tokens * 40);
    assert!(grammar.route_attempts <= contract.route_attempts);
    assert!(downshifted.declared_cost < source.declared_cost);
    assert!(downshifted.execution_calls > grammar.execution_calls);
}

#[test]
fn author_bench_records_stable_cassette_hashes() {
    let mut cx = cx();
    let report = run_author_bench(
        &mut cx,
        &standard_author_cases(),
        &[
            AuthorArm::SourcePayload,
            AuthorArm::ContractPayload,
            AuthorArm::ContractGrammar,
            AuthorArm::Downshifted,
        ],
    )
    .unwrap();

    let hashes = report
        .arms
        .iter()
        .map(|(arm, metrics)| (arm.clone(), metrics.cassette_hashes.clone()))
        .collect::<Vec<_>>();

    assert_eq!(
        hashes,
        vec![
            (
                AuthorArm::SourcePayload,
                strings(&[
                    "fnv1a64:ae559f705a45894a",
                    "fnv1a64:be7c1d0f87a8e045",
                    "fnv1a64:72d7e0bd6933926b",
                    "fnv1a64:2549e03c9f23b31c",
                ]),
            ),
            (
                AuthorArm::ContractPayload,
                strings(&[
                    "fnv1a64:87544d19766847fb",
                    "fnv1a64:bb3204ed718554d2",
                    "fnv1a64:cc636775dca55176",
                    "fnv1a64:adc9616d0bba5732",
                ]),
            ),
            (
                AuthorArm::ContractGrammar,
                strings(&[
                    "fnv1a64:0e1cbc9995fdc2f6",
                    "fnv1a64:f9aabee08c7b9ec2",
                    "fnv1a64:0f3f0e798b70da30",
                    "fnv1a64:3145fbe2521e8d92",
                ]),
            ),
            (
                AuthorArm::Downshifted,
                strings(&[
                    "fnv1a64:3dfc4886b420908a",
                    "fnv1a64:91d1e465e6e2d79b",
                    "fnv1a64:e0c7daa5abd12ec9",
                    "fnv1a64:4ab930cafebc96eb",
                ]),
            ),
        ]
    );
}

#[test]
fn author_bench_rejects_empty_inputs() {
    let mut cx = cx();
    let cases = standard_author_cases();

    assert!(run_author_bench(&mut cx, &[], &standard_author_arms()).is_err());
    assert!(run_author_bench(&mut cx, &cases, &[]).is_err());
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
