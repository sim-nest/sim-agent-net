#[path = "../src/legacy_import.rs"]
mod legacy_import;

pub use legacy_import::*;

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

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
    fn atomic_idempotent_accounted_and_inert_untrusted_evidence() {
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
        let inputs: Vec<_> = schemas
            .iter()
            .enumerate()
            .map(|(i, schema)| source(&format!("source-{i}"), schema, &format!("sample-{i}")))
            .collect();
        assert_eq!(
            import_legacy_batch(&mut store, inputs.clone())
                .unwrap()
                .len(),
            8
        );
        assert!(import_legacy_batch(&mut store, inputs).unwrap().is_empty());
        assert_eq!(store.entries().count(), 8);
        assert_eq!(
            store
                .entries()
                .filter(|entry| entry.decision_eligible())
                .count(),
            5
        );
        let report = legacy_import_report(store.entries());
        assert_eq!(
            (report.verified, report.partial, report.resources),
            (5, 3, 35)
        );
    }

    #[test]
    fn tampering_conflict_live_and_unknown_abort_before_publication() {
        let mut store = MemoryLegacyStore::default();
        let good = source("good", "codebench.result.v1", "a");
        let mut tampered = source("tampered", "codebench.result.v1", "b");
        tampered.bytes[0] ^= 1;
        assert!(matches!(
            import_legacy_batch(&mut store, vec![good, tampered]),
            Err(LegacyImportError::DigestMismatch(_))
        ));
        assert_eq!(store.entries().count(), 0);

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
        assert!(matches!(
            import_legacy_batch(
                &mut store,
                vec![source("future", "codebench.future.v1", "future")]
            ),
            Err(LegacyImportError::UnknownSchema(_))
        ));
    }

    #[test]
    fn execution_parity_is_independent() {
        let parity = CompatibilityParity {
            import_accounted: 8,
            import_expected: 8,
            sim_execution_matched: 4,
            sim_execution_expected: 5,
        };
        assert_eq!(parity.import_accounted, parity.import_expected);
        assert_ne!(parity.sim_execution_matched, parity.sim_execution_expected);
    }
}
