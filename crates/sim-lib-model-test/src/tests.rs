use crate::*;
use sim_kernel::{Expr, Symbol};
use sim_lib_provider::{
    EndpointCard, HarnessCard, PrincipalCard, ProviderFamilyCard, ProviderSeatCard, ProviderSeatId,
    ProviderSeatLimits,
};

fn fixture(
    label: &str,
    transport: &str,
    semantics: &str,
) -> (ProviderFamilyCard, ProviderSeatCard) {
    let family_id = Symbol::qualified("provider", format!("fixture-{label}"));
    let family = ProviderFamilyCard {
        family: family_id.clone(),
        transport: Symbol::new(transport),
        semantics: Symbol::new(semantics),
        auth_owner: Symbol::new("fixture"),
        wires: vec![Symbol::new("fixture-wire")],
        operations: vec![],
        revision: Expr::String("provider-r1".into()),
        extra: vec![],
    };
    let seat = ProviderSeatCard {
        seat: ProviderSeatId::new(family_id.clone(), label).unwrap(),
        family: family_id,
        principal: PrincipalCard {
            label: "ACCOUNT-SECRET".into(),
            kind: Symbol::new("fixture"),
            source: Symbol::new("fixture"),
            digest: "PRINCIPAL-SECRET".into(),
            extra: vec![],
        },
        endpoint: EndpointCard {
            address: "https://ENDPOINT-SECRET.invalid/token".into(),
            transport: Symbol::new(transport),
            revision: Expr::Nil,
            extra: vec![],
        },
        harness: Some(HarnessCard {
            kind: Symbol::new("broker"),
            label: "/EXECUTABLE/SECRET".into(),
            revision: Expr::String("h1".into()),
            extra: vec![],
        }),
        model: Some("model-a".into()),
        limits: ProviderSeatLimits::default(),
        revision: Expr::String("seat-r1".into()),
        extra: vec![],
    };
    (family, seat)
}

fn candidate(label: &str, transport: &str, semantics: &str) -> CandidateRevision {
    let (family, seat) = fixture(label, transport, semantics);
    CandidateRevision::from_provider(
        &family,
        &seat,
        ArtifactEvidence::Digest {
            algorithm: "sha256".into(),
            value: "ollama-digest-1".into(),
        },
        "backend-a",
        IdentityConfidence::Artifact,
    )
    .unwrap()
}

#[test]
fn provider_route_fixtures_are_distinct_and_agent_brokers_grade_the_product() {
    let local = candidate("local", "local-service", "model-turn");
    let direct = candidate("direct", "https", "model-turn");
    let model = candidate("broker-model", "broker-process", "model-turn");
    let agent = candidate("broker-agent", "broker-process", "agent-task");
    assert_eq!(local.route.semantics, RouteSemantics::LocalService);
    assert_eq!(direct.route.semantics, RouteSemantics::DirectApi);
    assert_eq!(model.route.semantics, RouteSemantics::BrokeredModel);
    assert_eq!(agent.route.semantics, RouteSemantics::BrokeredAgent);
    assert_eq!(agent.confidence, IdentityConfidence::Product);
    let ids = [&local, &direct, &model, &agent].map(|v| v.subject_revision().unwrap());
    for left in 0..ids.len() {
        for right in left + 1..ids.len() {
            assert_ne!(ids[left], ids[right]);
        }
    }
}

#[test]
fn every_identity_fact_changes_id_but_endpoint_alias_and_trial_harness_do_not() {
    let base = candidate("base", "https", "model-turn");
    for changed in [
        {
            let mut v = base.clone();
            v.artifact = ArtifactEvidence::Epoch("epoch-2".into());
            v
        },
        {
            let mut v = base.clone();
            v.backend = "backend-b".into();
            v
        },
        {
            let mut v = base.clone();
            v.external_harness_revision = Some("broker:h2".into());
            v
        },
        {
            let mut v = base.clone();
            v.quantization = Some("q4".into());
            v
        },
    ] {
        assert_ne!(
            base.subject_revision().unwrap(),
            changed.subject_revision().unwrap()
        );
    }
    let (family, mut alias) = fixture("base", "https", "model-turn");
    alias.endpoint.address = "https://another-alias.invalid".into();
    let same = CandidateRevision::from_provider(
        &family,
        &alias,
        base.artifact.clone(),
        "backend-a",
        IdentityConfidence::Artifact,
    )
    .unwrap();
    assert_eq!(
        base.subject_revision().unwrap(),
        same.subject_revision().unwrap()
    );
    let trial_harness_a = "study-harness-a";
    let trial_harness_b = "study-harness-b";
    assert_ne!(trial_harness_a, trial_harness_b);
    assert_eq!(
        base.subject_revision().unwrap(),
        same.subject_revision().unwrap()
    );
}

#[test]
fn seats_never_collapse_and_mismatch_is_quarantined_without_leaking_identity() {
    let left = candidate("seat-a", "https", "model-turn");
    let right = candidate("seat-b", "https", "model-turn");
    assert_ne!(
        left.subject_revision().unwrap(),
        right.subject_revision().unwrap()
    );
    let IdentityVerification::Quarantined(error) = verify_observed_identity(&left, &right).unwrap()
    else {
        panic!("mismatch accepted")
    };
    let diagnostic = error.to_string();
    assert!(!diagnostic.contains("seat-a"));
    assert!(!diagnostic.contains("seat-b"));
}

#[test]
fn hosted_alias_requires_epoch_and_snapshot_is_redacted_and_append_preserving() {
    let (family, seat) = fixture("hosted", "https", "model-turn");
    assert!(
        CandidateRevision::from_provider(
            &family,
            &seat,
            ArtifactEvidence::Epoch(String::new()),
            "backend",
            IdentityConfidence::Provider
        )
        .is_err()
    );
    let first = candidate("seat-a", "https", "model-turn");
    let second = candidate("seat-b", "https", "model-turn");
    let mut census = CandidateCensus::default();
    census.successful_sync(vec![first.clone(), second]).unwrap();
    census.successful_sync(vec![first]).unwrap();
    assert_eq!(
        census
            .records()
            .filter(|r| r.presence == CandidatePresence::Absent)
            .count(),
        1
    );
    let forbidden = [
        "ACCOUNT-SECRET",
        "PRINCIPAL-SECRET",
        "ENDPOINT-SECRET",
        "/EXECUTABLE/SECRET",
    ];
    let snapshot = OfflineSnapshot::from_census(&census, &forbidden).unwrap();
    for secret in forbidden {
        assert!(!snapshot.as_str().contains(secret));
    }
    assert!(snapshot.as_str().contains("Absent"));
}
