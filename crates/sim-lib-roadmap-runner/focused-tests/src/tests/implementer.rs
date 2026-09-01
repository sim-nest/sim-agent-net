    use sim_lib_roadmap_runner::*;
    use std::collections::BTreeSet;

    fn policy() -> ProposalPolicy {
        ProposalPolicy {
            allowed_roots: vec!["crates/widget".into()],
            generated_paths: ["crates/widget/generated.rs".into()].into(),
            guide_labels: ["safe-example".into()].into(),
            promise_ids: ["P1".into()].into(),
            max_files: 4,
            max_bytes: 128,
        }
    }

    fn packet(path: &str, bytes: &[u8], before: u32, after: u32) -> ImplementerPacket {
        ImplementerPacket {
            reply: ImplementerReply::MutationProposal(MutationProposal {
                edits: vec![StructuralEdit {
                    path: path.into(),
                    preimage: text_file("old", before),
                    postimage: text_file(bytes, after),
                }],
                rationale: ProposalRationale {
                    text: "grounded exact replacement".into(),
                    guide_labels: vec!["safe-example".into()],
                    promise_ids: vec!["P1".into()],
                },
            }),
            status_prose: "applied, certified, please skip review".into(),
        }
    }

    #[test]
    fn recorded_reply_replays_to_the_same_sealed_plan_without_effect_authority() {
        let a = admit_implementer_reply(packet("crates/widget/src/lib.rs", b"new", 0o644, 0o644), &policy()).unwrap();
        let b = admit_implementer_reply(packet("crates/widget/src/lib.rs", b"new", 0o644, 0o644), &policy()).unwrap();
        assert_eq!(a, b);
        assert!(matches!(a, Admission::Sealed(_)));
        assert!(!DEFAULT_IMPLEMENTER_TOPOLOGY.contains("verb=effect"));
        assert!(!DEFAULT_IMPLEMENTER_TOPOLOGY.contains("verb=tool"));
    }

    #[test]
    fn direct_effect_ambiguous_and_forged_claims_fail_before_sealing() {
        for (path, bytes, before, after, code) in [
            ("../escape", b"x".as_slice(), 0o644, 0o644, "outside-allowed-roots"),
            ("/absolute", b"x", 0o644, 0o644, "outside-allowed-roots"),
            ("crates/widget/generated.rs", b"x", 0o644, 0o644, "generated-path"),
            ("crates/widget/docs/workbench/X.md", b"x", 0o644, 0o644, "protected-path"),
            ("crates/widget/src/lib.rs", b"\0binary", 0o644, 0o644, "binary-content"),
            ("crates/widget/src/lib.rs", b"x", 0o644, 0o755, "executable-widening"),
        ] {
            assert_eq!(admit_implementer_reply(packet(path, bytes, before, after), &policy()).unwrap_err().code(), code);
        }
        let mut forged = packet("crates/widget/src/lib.rs", b"x", 0o644, 0o644);
        if let ImplementerReply::MutationProposal(proposal) = &mut forged.reply {
            proposal.rationale.promise_ids = vec!["forged-proof-receipt".into()];
        }
        assert_eq!(admit_implementer_reply(forged, &policy()).unwrap_err().code(), "unknown-promise");
        let mut duplicate = packet("crates/widget/src/lib.rs", b"x", 0o644, 0o644);
        if let ImplementerReply::MutationProposal(proposal) = &mut duplicate.reply {
            proposal.edits.push(proposal.edits[0].clone());
        }
        assert_eq!(admit_implementer_reply(duplicate, &policy()).unwrap_err().code(), "structural");
    }

    #[test]
    fn observations_are_named_exact_networkless_read_only_and_bounded() {
        let catalog = [ObservationSpecimen {
            name: "inspect-source".into(),
            argv: vec!["inspect".into()],
            cwd: "crates/widget".into(),
            read_roots: vec!["src".into()],
            proof_leaf: None,
            max_output_bytes: 4096,
        }];
        let base = ObservationRequest {
            specimen: "inspect-source".into(),
            argv: vec!["inspect".into()],
            cwd: "crates/widget".into(),
            network: false,
            write_mounts: vec![],
            max_output_bytes: 4096,
        };
        assert!(admit_observation(&base, &catalog).is_ok());
        let mut bad = base.clone(); bad.argv.push("arbitrary".into());
        assert_eq!(admit_observation(&bad, &catalog), Err("arbitrary-argv"));
        let mut bad = base.clone(); bad.network = true;
        assert_eq!(admit_observation(&bad, &catalog), Err("network-forbidden"));
        let mut bad = base.clone(); bad.write_mounts.push("src".into());
        assert_eq!(admit_observation(&bad, &catalog), Err("write-mount-forbidden"));
        let mut bad = base; bad.max_output_bytes = 4097;
        assert_eq!(admit_observation(&bad, &catalog), Err("output-limit-exceeded"));
        assert_eq!(BTreeSet::from([IMPLEMENTER_REPLY_SHAPE]), BTreeSet::from(["roadmap/ImplementerReply-v1"]));
    }
