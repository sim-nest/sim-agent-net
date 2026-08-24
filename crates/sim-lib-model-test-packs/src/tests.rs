use super::*;
use sim_study_core::EvidenceClass;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

fn selection(name: &str, stage: SelectionStage, tasks: &[&str]) -> SelectionRevision {
    SelectionRevision {
        name: name.into(),
        revision: format!("{name}-r1"),
        stage,
        task_revisions: tasks.iter().map(|x| (*x).into()).collect(),
    }
}
fn manifest(privacy: PackPrivacy) -> PackManifest {
    PackManifest {
        schema: PACK_SCHEMA.into(),
        id: "public/conformance".into(),
        revision: "epoch-r1".into(),
        families: vec![
            "deterministic".into(),
            "generated".into(),
            "judged".into(),
            "workspace".into(),
        ],
        facets: vec!["correctness".into()],
        evidence_class: if privacy == PackPrivacy::Public {
            EvidenceClass::Publishable
        } else {
            EvidenceClass::PrivateLocal
        },
        privacy,
        sources: vec![SourceObject {
            repository: "sim-nest/sim-agent-net".into(),
            commit: "0123456789abcdef".into(),
            tree: "tree-id".into(),
        }],
        licenses: vec![LicenseGrant {
            expression: "MPL-2.0".into(),
            notice_file: None,
        }],
        toolchain: "rust-1.89.0".into(),
        lockfile: ClosureFile {
            path: "Cargo.lock".into(),
            blob: "blob-lock".into(),
        },
        closure: vec![
            ClosureFile {
                path: "Cargo.lock".into(),
                blob: "blob-lock".into(),
            },
            ClosureFile {
                path: "tests/public.txt".into(),
                blob: "blob-test".into(),
            },
        ],
        public_tests: vec!["tests/public.txt".into()],
        grader_ids: vec!["sha256:grader".into()],
        hidden_grader_ids: vec![],
        bounds: WorkBounds {
            max_tasks: 8,
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_tool_calls: 8,
        },
        selections: vec![
            selection("smoke", SelectionStage::Smoke, &["scalar-r1"]),
            selection(
                "screen",
                SelectionStage::Screen,
                &["scalar-r1", "generated-r1"],
            ),
            selection(
                "confirmation",
                SelectionStage::Confirmation,
                &["scalar-r1", "generated-r1", "judged-r1"],
            ),
            selection(
                "full-reproduction",
                SelectionStage::FullReproduction,
                &["scalar-r1", "generated-r1", "judged-r1", "workspace-r1"],
            ),
        ],
    }
}

#[test]
fn public_conformance_pack_covers_all_protocols_and_exact_selections() {
    let pack = manifest(PackPrivacy::Public);
    pack.validate_shape().unwrap();
    let epoch = FixtureEpoch {
        id: "sha256:epoch".into(),
        files: BTreeMap::from([
            ("Cargo.lock".into(), b"lock".to_vec()),
            ("tests/public.txt".into(), b"test".to_vec()),
        ]),
    };
    let mut registry = PackRegistry::default();
    registry.register_public(pack.clone(), epoch).unwrap();
    assert_eq!(registry.len(), 1);
    assert!(registry.get("public/conformance", "epoch-r1").is_some());
    assert_eq!(
        pack.selections.iter().map(|x| x.stage).collect::<Vec<_>>(),
        [
            SelectionStage::Smoke,
            SelectionStage::Screen,
            SelectionStage::Confirmation,
            SelectionStage::FullReproduction
        ]
    );
    assert_eq!(pack.export_public().unwrap().grader_ids, ["sha256:grader"]);
}

#[test]
fn private_bytes_never_export_or_register() {
    let mut private = manifest(PackPrivacy::PrivateLocal);
    private.hidden_grader_ids.push("sha256:hidden".into());
    assert_eq!(private.export_public(), Err(PackError::PrivateExport));
    let epoch = FixtureEpoch {
        id: "x".into(),
        files: BTreeMap::new(),
    };
    assert!(matches!(
        PackRegistry::default().register_public(private, epoch),
        Err(PackError::PrivateExport)
    ));
}

#[test]
fn privacy_classes_cover_every_flow_boundary() {
    let fields = PackManifest::field_classes();
    assert!(fields.contains(&(
        "hidden_grader_ids",
        sim_study_core::FieldClass::PrivateLocal
    )));
    assert!(fields.contains(&("sources", sim_study_core::FieldClass::DigestOnly)));
    for boundary in [
        "manifest-load",
        "prepared-trial",
        "evidence",
        "report",
        "export",
    ] {
        assert!(!boundary.is_empty());
        assert_eq!(
            manifest(PackPrivacy::PrivateLocal).export_public(),
            Err(PackError::PrivateExport)
        );
    }
}

#[test]
fn pinned_git_objects_are_authority_and_closure_changes_epoch() {
    let root = temp_repo();
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::write(root.join("Cargo.lock"), b"v1\n").unwrap();
    fs::write(root.join("tests/public.txt"), b"fixture\n").unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=Pack Test",
            "-c",
            "user.email=pack@example.invalid",
            "commit",
            "-m",
            "epoch one",
        ],
    );
    let commit = text(&root, &["rev-parse", "HEAD"]);
    let tree = text(&root, &["rev-parse", "HEAD^{tree}"]);
    let closure = ["Cargo.lock", "tests/public.txt"]
        .map(|path| ClosureFile {
            path: path.into(),
            blob: text(&root, &["rev-parse", &format!("HEAD:{path}")]),
        })
        .to_vec();
    let source = SourceObject {
        repository: "local/test".into(),
        commit: commit.clone(),
        tree,
    };
    let first = FixtureEpoch::import(&root, &source, &closure).unwrap();
    first.verify_regeneration(&root, &source, &closure).unwrap();
    fs::write(
        root.join("tests/public.txt"),
        b"mutable checkout is ignored\n",
    )
    .unwrap();
    assert_eq!(
        first,
        FixtureEpoch::import(&root, &source, &closure).unwrap()
    );
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=Pack Test",
            "-c",
            "user.email=pack@example.invalid",
            "commit",
            "-m",
            "epoch two",
        ],
    );
    let commit2 = text(&root, &["rev-parse", "HEAD"]);
    let tree2 = text(&root, &["rev-parse", "HEAD^{tree}"]);
    let closure2 = ["Cargo.lock", "tests/public.txt"]
        .map(|path| ClosureFile {
            path: path.into(),
            blob: text(&root, &["rev-parse", &format!("HEAD:{path}")]),
        })
        .to_vec();
    let second = FixtureEpoch::import(
        &root,
        &SourceObject {
            repository: "local/test".into(),
            commit: commit2,
            tree: tree2,
        },
        &closure2,
    )
    .unwrap();
    assert_ne!(first.id, second.id);
    let _ = fs::remove_dir_all(root);
}

fn temp_repo() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "sim-pack-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    root
}
fn git(root: &std::path::Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
}
fn text(root: &std::path::Path, args: &[&str]) -> String {
    String::from_utf8(
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .into()
}
