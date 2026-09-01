use super::*;
use sim_lib_exec::{SandboxEvidence, SandboxReport, SandboxResult};
use sim_source_deck::{IndexAnchor, IndexFragment, IndexSpecimen};

struct Launcher;
impl SandboxLauncher for Launcher {
    fn id(&self) -> &str {
        "fixture"
    }
    fn launch(&self, request: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
        SandboxAttempt::Completed(SandboxResult {
            stdout: b"artifact".to_vec(),
            stderr: vec![],
            exit_code: 0,
            report: SandboxReport {
                launcher: "fixture".into(),
                controls: request
                    .policy
                    .requirements()
                    .keys()
                    .map(|control| SandboxEvidence {
                        control: *control,
                        achieved: true,
                        detail: "fixture proof".into(),
                    })
                    .collect(),
                limit_hits: vec![],
                cleanup: "complete".into(),
            },
        })
    }
}
struct Repo(BTreeMap<String, Vec<u8>>);
impl SourceRepository for Repo {
    fn read_regular(
        &self,
        _: &RepositoryIdentity,
        path: &str,
    ) -> Result<Vec<u8>, SourceDeckProviderError> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| SourceDeckProviderError::Read(path.into()))
    }
}
struct Fragment;
impl FragmentDecoder for Fragment {
    fn decode(&self, _: &[u8]) -> Result<IndexFragment, sim_source_deck::Failure> {
        Ok(IndexFragment {
            owner: "owner".into(),
            anchors: vec![IndexAnchor {
                id: "anchor/private".into(),
                owner: "owner".into(),
                source_path: Some("src/lib.rs".into()),
            }],
            specimens: vec![IndexSpecimen {
                id: "spec/full-signature".into(),
                owner: "owner".into(),
            }],
        })
    }
}
struct Artifact;
impl RepoContractDecoder for Artifact {
    fn decode(&self, bytes: &[u8]) -> Result<ArtifactProjection, SourceDeckProviderError> {
        let fragment_bytes = b"fragment".to_vec();
        let fragment_id = ByteContentId::of(&fragment_bytes).unwrap();
        let mut certificate = ClaimCertificate {
            anchor: "anchor/private".into(),
            owner: "owner".into(),
            fragment_id: fragment_id.clone(),
            digest: ByteContentId::of(b"placeholder").unwrap(),
        };
        certificate.digest = certificate.expected_digest().unwrap();
        Ok(ArtifactProjection {
            repository_head: "head-1".into(),
            fragment: FragmentPin {
                owner: "owner".into(),
                bytes: fragment_bytes,
                content_id: fragment_id,
            },
            certificates: vec![certificate],
            source_paths: vec!["src/lib.rs".into()],
            excerpts: vec![ArtifactExcerpt {
                id: "excerpt/private-signature".into(),
                path: "src/lib.rs".into(),
                start: 0,
                end: 18,
            }],
            specimens: vec![ArtifactSpecimen {
                id: "spec/full-signature".into(),
                path: "tests/signature.txt".into(),
            }],
            limitations: vec![],
            artifact_id: ByteContentId::of(bytes).unwrap(),
        })
    }
}
fn request() -> SourceDeckRequest {
    SourceDeckRequest {
        repository: RepositoryIdentity {
            owner: "owner".into(),
            repository: "repo".into(),
            local_head: "head-1".into(),
            root: "root-ref".into(),
        },
        allowed_roots: vec!["src".into(), "tests".into()],
        queries: vec![
            SourceQuery::Anchor("anchor/private".into()),
            SourceQuery::Excerpt("excerpt/private-signature".into()),
            SourceQuery::Specimen("spec/full-signature".into()),
        ],
        artifact_command: ArtifactCommand {
            identity: "xtask-repo-contract-v1".into(),
            program: "cargo".into(),
            argv: vec![
                "run".into(),
                "-p".into(),
                "xtask".into(),
                "--".into(),
                "repo-contract".into(),
                "--emit-stdout".into(),
                "--repo".into(),
                SOURCE_GUEST_ROOT.into(),
            ],
            working_directory: SOURCE_GUEST_ROOT.into(),
            environment: BTreeMap::from([("PATH".into(), "/toolchain/bin".into())]),
        },
        bounds: SourceDeckBounds {
            command_wall_time_ms: 10_000,
            command_output_bytes: 4096,
            deck: DeckLimits::strict(1, 1, 4, 4, 4, 4, 4096),
        },
    }
}

#[test]
fn grounds_private_declaration_and_full_signature_with_real_receipts() {
    let repo = Repo(BTreeMap::from([
        (
            "src/lib.rs".into(),
            b"fn private(a: u32) -> bool { true }".to_vec(),
        ),
        (
            "tests/signature.txt".into(),
            b"fn private(a: u32) -> bool".to_vec(),
        ),
    ]));
    let receipt = SourceDeckProvider::new(&Launcher, &repo, &Artifact, &Fragment)
        .provide(&request(), &ProcessCancellation::default())
        .unwrap();
    assert_eq!(receipt.artifact.command_identity, "xtask-repo-contract-v1");
    assert_eq!(receipt.artifact.repository_head, "head-1");
    assert_eq!(receipt.deck.evidence().len(), 3);
    assert!(receipt.dependencies.contains("src/lib.rs"));
}

#[test]
fn mutation_invalidation_is_dependency_exact() {
    let repo = Repo(BTreeMap::from([
        (
            "src/lib.rs".into(),
            b"fn private(a: u32) -> bool { true }".to_vec(),
        ),
        ("tests/signature.txt".into(), b"signature".to_vec()),
    ]));
    let receipt = SourceDeckProvider::new(&Launcher, &repo, &Artifact, &Fragment)
        .provide(&request(), &ProcessCancellation::default())
        .unwrap();
    assert!(!receipt.reusable_after(&TouchedPaths(BTreeSet::from(["src/lib.rs".into()]))));
    assert!(receipt.reusable_after(&TouchedPaths(BTreeSet::from(["README.md".into()]))));
    assert!(!receipt.reusable_after(&TouchedPaths(BTreeSet::from([
        "docs/generated/repo-contract.json".into()
    ]))));
}

#[test]
fn rejects_traversal_absolute_symlink_escape_devices_removed_and_renamed_paths() {
    for path in ["../secret", "/etc/passwd", "src/../secret", "src\\secret"] {
        assert!(validate_path(path, &["src".into()]).is_err(), "{path}");
    }
    let missing = Repo(BTreeMap::new());
    let err = SourceDeckProvider::new(&Launcher, &missing, &Artifact, &Fragment)
        .provide(&request(), &ProcessCancellation::default())
        .unwrap_err();
    assert!(matches!(err, SourceDeckProviderError::Read(_)));
}

#[test]
fn rejects_malformed_scanner_output_head_mismatch_and_oversize() {
    struct Bad;
    impl RepoContractDecoder for Bad {
        fn decode(&self, _: &[u8]) -> Result<ArtifactProjection, SourceDeckProviderError> {
            Err(SourceDeckProviderError::Invalid(
                "malformed fragment".into(),
            ))
        }
    }
    let repo = Repo(BTreeMap::new());
    assert!(matches!(
        SourceDeckProvider::new(&Launcher, &repo, &Bad, &Fragment)
            .provide(&request(), &ProcessCancellation::default()),
        Err(SourceDeckProviderError::Invalid(_))
    ));
    let mut small = request();
    small.bounds.command_output_bytes = 4;
    assert!(matches!(
        SourceDeckProvider::new(&Launcher, &repo, &Artifact, &Fragment)
            .provide(&small, &ProcessCancellation::default()),
        Err(SourceDeckProviderError::Oversize)
    ));
    let mut wrong = request();
    wrong.repository.local_head = "head-2".into();
    assert!(matches!(
        SourceDeckProvider::new(&Launcher, &repo, &Artifact, &Fragment)
            .provide(&wrong, &ProcessCancellation::default()),
        Err(SourceDeckProviderError::HeadMismatch)
    ));
}

#[test]
fn command_is_literal_read_only_networkless_and_environment_sealed() {
    let built = sandbox_request(&request()).unwrap();
    assert_eq!(
        built.argv.iter().map(ArgAtom::as_str).collect::<Vec<_>>(),
        request().artifact_command.argv
    );
    assert_eq!(built.policy.mounts()[0].access, MountAccess::ReadOnly);
    assert_eq!(
        built.policy.requirements()[&SandboxControl::Network],
        SandboxRequirement::Required
    );
    assert_eq!(built.policy.limits().output_bytes, 4096);
}
