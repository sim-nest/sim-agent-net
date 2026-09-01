#![forbid(unsafe_code)]

#[path = "../src/assistance.rs"]
mod assistance;
#[path = "../src/domain.rs"]
mod domain;
#[path = "../src/external.rs"]
mod external;
#[path = "../src/generated.rs"]
mod generated;

pub use assistance::*;
pub use domain::*;
pub use external::*;
pub use generated::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    Schema,
    Bounds,
    Duplicate(&'static str),
    Missing(&'static str),
    Determinism,
    Repository,
    ObjectMismatch(String),
}

#[cfg(test)]
mod portfolio_tests {
    use super::*;

    #[test]
    fn generated_pairs_recreate_and_mutations_fail() {
        let bounds = GenerationBounds { max_depth: 4, max_entities: 4, max_operations: 3, max_prompt_bytes: 256 };
        for family in [GeneratedFamily::SymbolicTree, GeneratedFamily::StateTrace, GeneratedFamily::ConstraintPlan, GeneratedFamily::CausalDebug] {
            let pair = generate_pair(family, 17, bounds.clone()).unwrap();
            assert_eq!(pair, generate_pair(family, 17, bounds.clone()).unwrap());
            assert!(pair.0.verify(&pair.0.expected_answer));
            assert!(!pair.0.verify("mutated"));
            assert_eq!((&pair.0.facts, &pair.0.operations, &pair.0.entities, &pair.0.answer_schema, &pair.0.renderer, pair.0.prompt_bytes), (&pair.1.facts, &pair.1.operations, &pair.1.entities, &pair.1.answer_schema, &pair.1.renderer, pair.1.prompt_bytes));
            assert_ne!(pair.0.dependency_wiring, pair.1.dependency_wiring);
        }
    }

    #[test]
    fn reports_stay_family_scoped_and_external_content_is_bound() {
        let starters = public_starter_domains();
        assert_eq!(starters.len(), 6);
        assert!(starters.iter().all(|domain| domain.validate().is_ok() && domain.pack_ids.len() >= 2 && domain.facets.len() == 3));
        let trials = [PairTrial { family: GeneratedFamily::SymbolicTree, pair_id: "a".into(), depth: 2, target: TrialState::Fail, control: TrialState::Pass }, PairTrial { family: GeneratedFamily::SymbolicTree, pair_id: "b".into(), depth: 3, target: TrialState::Pass, control: TrialState::Fail }];
        let report = capability_by_family(&trials, 0.5);
        assert_eq!(report.len(), 1);
        assert_eq!(report[&GeneratedFamily::SymbolicTree].inversions, 1);
        assert_eq!(report[&GeneratedFamily::SymbolicTree].threshold, Some(3));
        assert!(guard_language_pack("replacement runtime").is_err());
        let bundle = ExternalEvaluatorBundle::seal(ExternalFormat::EvalPlus, "evalplus/v0.3".into(), vec![ExternalCase { id: "humaneval/0".into(), input_digest: "sha256:in".into(), expected_digest: "sha256:out".into() }]).unwrap();
        bundle.validate().unwrap();
        assert!(!bundle.isolation_trusted);
    }
}
