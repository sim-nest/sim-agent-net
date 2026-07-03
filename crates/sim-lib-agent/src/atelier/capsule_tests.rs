use super::capsule::{
    CapsuleJobOutcome, GeneratedArtifact, JobSite, PinPlanEntry, fake_change_capsule,
    review_change_capsule,
};

#[test]
fn successful_capsule_replays_and_accepts() {
    let capsule = fake_change_capsule().unwrap();
    let review = review_change_capsule(&capsule).unwrap();

    assert!(review.accepted, "{:?}", review.failure_reasons);
    assert_eq!(review.content_hash, review.replay_content_hash);
    assert!(review.preview_repos.contains(&"sim-agent-net".to_owned()));
}

#[test]
fn failed_validation_rejects_capsule() {
    let mut capsule = fake_change_capsule().unwrap();
    capsule.validations[0].outcome = CapsuleJobOutcome::Failed;

    let review = review_change_capsule(&capsule).unwrap();
    assert!(!review.accepted);
    assert!(
        review
            .failure_reasons
            .iter()
            .any(|reason| reason.contains("validation job"))
    );
}

#[test]
fn stale_pin_is_refused_before_pin_update() {
    let mut capsule = fake_change_capsule().unwrap();
    capsule.pin_plan = vec![PinPlanEntry {
        repo: "sim-agent-net".to_owned(),
        current_commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
        new_commit: "cccccccccccccccccccccccccccccccccccccccc".to_owned(),
        pushed_commit_exists: false,
    }];

    let review = review_change_capsule(&capsule).unwrap();
    assert!(!review.accepted);
    assert!(
        review
            .failure_reasons
            .iter()
            .any(|reason| reason.contains("pushed upstream commit"))
    );
}

#[test]
fn generated_doc_hand_edit_is_refused() {
    let mut capsule = fake_change_capsule().unwrap();
    capsule.generated_artifacts = vec![GeneratedArtifact {
        repo: "sim-web".to_owned(),
        path: "docs/generated/contract.md".to_owned(),
        generator: "manual edit".to_owned(),
        generated_public_doc: true,
        hand_edited: true,
    }];

    let review = review_change_capsule(&capsule).unwrap();
    assert!(!review.accepted);
    assert!(
        review
            .failure_reasons
            .iter()
            .any(|reason| reason.contains("must be regenerated"))
    );
}

#[test]
fn validation_jobs_must_be_realized_on_process_or_fabric_sites() {
    let mut capsule = fake_change_capsule().unwrap();
    capsule.validations[0].site = JobSite::LocalCoroutine;

    let review = review_change_capsule(&capsule).unwrap();
    assert!(!review.accepted);
    assert!(
        review
            .failure_reasons
            .iter()
            .any(|reason| reason.contains("process or fabric"))
    );
}
