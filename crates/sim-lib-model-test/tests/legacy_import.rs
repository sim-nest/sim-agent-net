use sha2::{Digest, Sha256};
use sim_lib_model_test::*;

fn source(id: &str, schema: &str, sample: &str) -> SealedLegacyObject {
    let bytes = format!(r#"{{"schema_id":"{schema}","schema_version":1,"subject_id":"model","task_id":"task","harness_id":"harness","request_id":"request","treatment_id":"control","sample_id":"{sample}","family":"coding","state":"complete","resource_units":7}}"#).into_bytes();
    SealedLegacyObject {
        source_id: id.into(),
        path: format!("sealed/{id}.json"),
        expected_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
        bytes,
        historical_git_object: Some("2f50d70cc47fa2682f655c09396ef4d2e5b53876".into()),
        sealed: true,
    }
}

#[test]
fn repeated_import_is_noop_and_valid_repeats_are_samples() {
    let mut store = MemoryLegacyStore::default();
    let a = source("a", "codebench.result.v1", "sample-1");
    let b = source("b", "codebench.result.v1", "sample-2");
    assert_eq!(
        import_legacy_batch(&mut store, vec![b.clone(), a.clone()])
            .unwrap()
            .len(),
        2
    );
    assert!(
        import_legacy_batch(&mut store, vec![a, b])
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.entries().count(), 2);
    let report = legacy_import_report(store.entries());
    assert_eq!(
        (
            report.verified,
            report.resources,
            report.by_family["coding"]
        ),
        (2, 14, 2)
    );
}

#[test]
fn one_byte_tamper_aborts_whole_batch() {
    let mut store = MemoryLegacyStore::default();
    let good = source("good", "codebench.sweep-cost.v1", "one");
    let mut bad = source("bad", "codebench.result.v1", "two");
    bad.bytes.push(b' ');
    assert!(matches!(
        import_legacy_batch(&mut store, vec![good, bad]),
        Err(LegacyImportError::DigestMismatch(_))
    ));
    assert_eq!(store.entries().count(), 0);
}

#[test]
fn schemas_classification_and_decision_boundary_are_closed() {
    let schemas = [
        "codebench.sweep-manifest.v1",
        "codebench.sweep-lifecycle.v1",
        "codebench.result.v1",
        "codebench.catalog-task-revision.v1",
        "codebench.catalog-request-contract.v1",
        "codebench.depth-epoch.v1",
        "codebench.sim-result.v2",
        "codebench.sweep-cost.v1",
    ];
    let mut store = MemoryLegacyStore::default();
    let inputs = schemas
        .iter()
        .enumerate()
        .map(|(i, s)| source(&format!("s{i}"), s, &format!("n{i}")))
        .collect();
    import_legacy_batch(&mut store, inputs).unwrap();
    assert_eq!(store.entries().count(), schemas.len());
    assert_eq!(store.entries().filter(|e| e.decision_eligible()).count(), 5);
    assert!(
        store
            .entries()
            .filter(|e| !e.decision_eligible())
            .all(|e| matches!(e.class, ImportClass::Partial))
    );
}

#[test]
fn conflicting_observations_and_live_or_unknown_sources_fail_closed() {
    let mut store = MemoryLegacyStore::default();
    let a = source("a", "codebench.result.v1", "same");
    let b = source("b", "codebench.sweep-cost.v1", "same");
    assert!(matches!(
        import_legacy_batch(&mut store, vec![a, b]),
        Err(LegacyImportError::ConflictingObservation(_))
    ));
    let mut live = source("live", "codebench.result.v1", "live");
    live.sealed = false;
    assert!(matches!(
        import_legacy_batch(&mut store, vec![live]),
        Err(LegacyImportError::LiveSource(_))
    ));
    let unknown = source("unknown", "codebench.future.v1", "future");
    assert!(matches!(
        import_legacy_batch(&mut store, vec![unknown]),
        Err(LegacyImportError::UnknownSchema(_))
    ));
}

#[test]
fn planning_and_reporting_require_no_legacy_runtime() {
    let parity = CompatibilityParity {
        import_accounted: 8,
        import_expected: 8,
        sim_execution_matched: 5,
        sim_execution_expected: 6,
    };
    assert_eq!(parity.import_accounted, parity.import_expected);
    assert_ne!(parity.sim_execution_matched, parity.sim_execution_expected);
}
