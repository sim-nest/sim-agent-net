use crate::protocol::*;
use std::{fs, path::PathBuf, process::Command, sync::Arc};

fn spec(protocol: &str, visible: &[u8], oracle: &[u8]) -> TaskRevisionSpec {
    TaskRevisionSpec {
        protocol: protocol.into(),
        visible_inputs: visible.into(),
        hidden_oracle: oracle.into(),
        parser_revision: "parser-r1".into(),
        grader_revision: "grader-r1".into(),
        judge_calibration: None,
        tools: vec!["tool/read".into()],
        toolchain: "rust-1".into(),
        seed: 7,
        closure: None,
    }
}

#[test]
fn third_party_protocol_loads_without_dispatch_changes_and_declares_contract() {
    struct ThirdParty(FakeProtocol);
    impl ModelTaskProtocol for ThirdParty {
        fn metadata(&self) -> &ProtocolMetadata {
            self.0.metadata()
        }
        fn output_shape(&self) -> OutputShape {
            self.0.output_shape()
        }
        fn effect_requirements(&self) -> std::collections::BTreeSet<EffectRequirement> {
            self.0.effect_requirements()
        }
        fn prepare(&self, t: &TaskRevision, s: u64) -> Result<PreparedTrial, ProtocolError> {
            self.0.prepare(t, s)
        }
        fn grade(&self, p: &PreparedTrial, r: &TerminalResponse) -> Vec<FacetObservation> {
            self.0.grade(p, r)
        }
    }
    let mut registry = ProtocolRegistry::default();
    registry
        .register(Arc::new(ThirdParty(FakeProtocol::new(
            "third-party",
            FakeProtocolKind::Scalar,
        ))))
        .unwrap();
    let loaded = registry.load("third-party").unwrap();
    let task = TaskRevision::seal(spec("third-party", b"42", b"private answer"));
    let prepared = loaded.prepare(&task, 99).unwrap();
    assert_eq!(prepared.seed, 99);
    assert_eq!(prepared.output_shape.id, "model-test/Scalar");
    assert!(prepared.effects.is_empty());
    assert!(
        loaded.grade(
            &prepared,
            &TerminalResponse {
                bytes: b"42".into(),
                tool_receipts: vec![]
            }
        )[0]
        .passed
    );
}

#[test]
fn task_revision_binds_every_input_and_never_exports_private_bytes() {
    let base = spec("scalar", b"question", b"TOP SECRET");
    let id = TaskRevision::seal(base.clone()).content_id().to_owned();
    let changes: Vec<TaskRevisionSpec> = vec![
        TaskRevisionSpec {
            visible_inputs: b"other".into(),
            ..base.clone()
        },
        TaskRevisionSpec {
            hidden_oracle: b"other".into(),
            ..base.clone()
        },
        TaskRevisionSpec {
            parser_revision: "p2".into(),
            ..base.clone()
        },
        TaskRevisionSpec {
            grader_revision: "g2".into(),
            ..base.clone()
        },
        TaskRevisionSpec {
            tools: vec!["other".into()],
            ..base.clone()
        },
        TaskRevisionSpec {
            toolchain: "new".into(),
            ..base.clone()
        },
        TaskRevisionSpec {
            seed: 8,
            ..base.clone()
        },
    ];
    for changed in changes {
        assert_ne!(id, TaskRevision::seal(changed).content_id());
    }
    let sealed = TaskRevision::seal(base);
    assert!(!format!("{sealed:?}").contains("TOP SECRET"));
}

#[test]
fn generated_pack_is_ranked_stable_unique_leak_free_and_oracles_agree() {
    let pack = seal_generated_pack(
        "gen-r1",
        4,
        vec![
            (0, "a".into(), b"public-a".into()),
            (1, "b".into(), b"public-b".into()),
        ],
        &[b"answer".to_vec()],
    )
    .unwrap();
    assert_eq!(pack.ranks, vec![0, 1]);
    assert!(matches!(
        seal_generated_pack(
            "g",
            0,
            vec![(1, "a".into(), vec![]), (0, "b".into(), vec![])],
            &[]
        ),
        Err(ProtocolError::UnstableTraversal)
    ));
    assert!(matches!(
        seal_generated_pack(
            "g",
            0,
            vec![(0, "a".into(), vec![]), (1, "a".into(), vec![])],
            &[]
        ),
        Err(ProtocolError::DuplicateId(_))
    ));
    assert!(matches!(
        seal_generated_pack(
            "g",
            0,
            vec![(0, "a".into(), b"the answer leaked".into())],
            &[b"answer".to_vec()]
        ),
        Err(ProtocolError::AnswerLeakage)
    ));
    assert!(require_oracle_agreement(&pack.member_ids, &pack.member_ids).is_ok());
    assert!(matches!(
        require_oracle_agreement(&pack.member_ids, &[]),
        Err(ProtocolError::OracleDisagreement)
    ));
}

#[test]
fn deterministic_failure_wins_and_disagreement_or_abstention_withholds() {
    let cal = JudgeCalibration {
        revision: "j1".into(),
        rubric: "semantic correctness".into(),
        anchors: vec!["sha256:a".into()],
        blinded: true,
        disagreement_limit: 100,
        abstention_allowed: false,
    };
    let votes = [
        JudgeVote {
            judge_revision: "a".into(),
            score: 0.9,
            abstained: false,
            rationale_id: "x".into(),
        },
        JudgeVote {
            judge_revision: "b".into(),
            score: 0.1,
            abstained: false,
            rationale_id: "y".into(),
        },
    ];
    assert_eq!(
        calibrated_judgment(&cal, "candidate-r1", &votes, false).evidence_class,
        EvidenceClass::Deterministic
    );
    assert_eq!(
        calibrated_judgment(&cal, "candidate-r1", &votes, true).evidence_class,
        EvidenceClass::Withheld
    );
    let abstain = [JudgeVote {
        abstained: true,
        ..votes[0].clone()
    }];
    assert_eq!(
        calibrated_judgment(&cal, "candidate-r1", &abstain, true).evidence_class,
        EvidenceClass::Withheld
    );
}

#[test]
fn keyword_only_wrong_semantics_fails_and_all_fixture_protocols_are_ready() {
    for (id, kind, response, pass) in [
        ("scalar", FakeProtocolKind::Scalar, b"7".as_slice(), true),
        (
            "generated",
            FakeProtocolKind::Generated,
            b"done".as_slice(),
            true,
        ),
        (
            "judged",
            FakeProtocolKind::JudgedDocument,
            b"rubric anchor therefore conclusion".as_slice(),
            true,
        ),
        (
            "mutation",
            FakeProtocolKind::WorkspaceMutation,
            b"changed".as_slice(),
            true,
        ),
    ] {
        let p = FakeProtocol::new(id, kind);
        let task = TaskRevision::seal(spec(
            id,
            if kind == FakeProtocolKind::Scalar {
                b"7"
            } else {
                b"prompt"
            },
            b"secret",
        ));
        let prepared = p.prepare(&task, 1).unwrap();
        let receipts = match kind {
            FakeProtocolKind::Generated => vec![task.content_id().into()],
            FakeProtocolKind::WorkspaceMutation => vec!["sha256:workspace".into()],
            _ => vec![],
        };
        assert_eq!(
            p.grade(
                &prepared,
                &TerminalResponse {
                    bytes: response.into(),
                    tool_receipts: receipts
                }
            )[0]
            .passed,
            pass
        );
    }
    let p = FakeProtocol::new("judged", FakeProtocolKind::JudgedDocument);
    let task = TaskRevision::seal(spec("judged", b"prompt", b"secret"));
    let got = p.grade(
        &p.prepare(&task, 0).unwrap(),
        &TerminalResponse {
            bytes: b"rubric anchor therefore keyword-only".into(),
            tool_receipts: vec![],
        },
    );
    assert!(!got[0].passed);
}

#[test]
fn repository_closure_fails_closed_but_ignores_unrelated_dirt_and_revisions_immutably() {
    let dir = std::env::temp_dir().join(format!("sim-model-task-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    run(&dir, &["init", "-q"]);
    run(&dir, &["config", "user.email", "fixture@example.invalid"]);
    run(&dir, &["config", "user.name", "Fixture"]);
    fs::write(dir.join("oracle.txt"), "one").unwrap();
    run(&dir, &["add", "oracle.txt"]);
    run(&dir, &["commit", "-qm", "one"]);
    let head = output(&dir, &["rev-parse", "HEAD"]);
    fs::write(dir.join("unrelated.txt"), "dirt").unwrap();
    let first = RepositoryClosure::seal(&dir, &head, [PathBuf::from("oracle.txt")]).unwrap();
    fs::write(dir.join("oracle.txt"), "two").unwrap();
    assert!(matches!(
        RepositoryClosure::seal(&dir, &head, [PathBuf::from("oracle.txt")]),
        Err(ProtocolError::ClosureDirty(_))
    ));
    run(&dir, &["add", "oracle.txt"]);
    run(&dir, &["commit", "-qm", "two"]);
    let next = output(&dir, &["rev-parse", "HEAD"]);
    let second = RepositoryClosure::seal(&dir, &next, [PathBuf::from("oracle.txt")]).unwrap();
    assert_ne!(first.identity(), second.identity());
    assert_eq!(first.commit, head);
    assert!(matches!(
        RepositoryClosure::seal(&dir, &next, [PathBuf::from("untracked")]),
        Err(ProtocolError::ClosureMissing(_)) | Err(ProtocolError::ClosureNotInCommit(_))
    ));
    let _ = fs::remove_dir_all(&dir);
}
fn run(dir: &std::path::Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success()
    )
}
fn output(dir: &std::path::Path, args: &[&str]) -> String {
    String::from_utf8(
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .into()
}
// conformance: versioned model task protocols.
