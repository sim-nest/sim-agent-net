    #[test]
    fn loaded_local_runner_has_exact_shapes_and_no_delivery_surface() {
        let manifest = LocalRoadmapRunnerLib::new().manifest();
        for verb in LOCAL_ROADMAP_VERBS {
            assert!(manifest.exports.iter().any(|export| matches!(export,
                sim_kernel::Export::Function { symbol, .. } if symbol == &Symbol::qualified("roadmap", verb))));
            for suffix in ["Args", "Result"] {
                assert!(manifest.exports.iter().any(|export| matches!(export,
                    sim_kernel::Export::Shape { symbol, .. } if symbol == &Symbol::qualified(format!("roadmap/{verb}"), suffix))));
            }
        }
        let exports = format!("{:?}", manifest.exports);
        for forbidden in ["push", "publish", "release", "closeout", "roadmap-status"] {
            assert!(!exports.contains(&format!("roadmap/{forbidden}")));
        }
    }

    #[test]
    fn local_runner_gates_mutation_and_replays_with_pinned_generation() {
        let port = PublicLocalRoadmapPort::default();
        let request = local_request("run", false);
        assert!(port.invoke(&request, GenerationHandle::acquire("sha256:generation")).unwrap_err().contains("disposable"));
        let observed = local_request("run", true);
        let handle = GenerationHandle::acquire("sha256:generation");
        let retained = handle.clone();
        let receipt = port.invoke(&observed, handle).unwrap();
        assert!(receipt.journal_acknowledged);
        assert_eq!(receipt.identity, observed.identity);
        let replay = local_request("replay", false);
        assert!(port.invoke(&replay, retained).unwrap().detail.contains("replayed"));
        assert!(port.invoke(&replay, GenerationHandle::acquire("sha256:new")).unwrap_err().contains("drift"));
    }

    #[test]
    fn false_signature_stays_unresolved_until_exact_correlated_proof() {
        use sim_roadmap_core::PromiseId;

        let authority = ProofAuthority {
            plan: proof_content(1),
            deck: proof_content(2),
            mutation: proof_content(3),
            launcher: "networkless-v1".into(),
            policy: proof_content(4),
            proof_definition: proof_content(5),
        };
        let promise = GroundedPromise {
            id: PromiseId::new("public-signature").unwrap(),
            admitted_proofs: [("exact-source".into(), proof_content(5))].into(),
            inconclusive_fallback: None,
        };
        let receipt = |proof: &str, disposition| CorrelatedProof {
            authority: authority.clone(),
            receipt: TypedProofReceipt {
                proof: proof.into(),
                effect_id: None,
                disposition,
                exit_code: Some(0),
                timeout: false,
                signal: None,
                truncated: false,
                launcher_identity: Some("networkless-v1".into()),
                sandbox_identity: Some("sandbox".into()),
                stdout_object: None,
                stderr_object: None,
                observed_at: "logical:1".into(),
                semantic_detail: "exact signature predicate".into(),
            },
            evidence: proof_content(9),
        };

        assert!(matches!(
            decide_promise(
                &promise,
                &authority,
                &receipt("generic-green", ProofDisposition::Passed),
                None,
                &mut 0,
            ),
            Err(AcceptanceFailure::UnadmittedProof(_))
        ));
        let refuted = decide_promise(
            &promise,
            &authority,
            &receipt("exact-source", ProofDisposition::Failed),
            None,
            &mut 0,
        )
        .unwrap();
        assert!(matches!(
            accept_all(std::slice::from_ref(&promise), &[refuted], &ParentAcceptance::default()),
            Err(AcceptanceFailure::Refuted(_))
        ));
        let exact = receipt("exact-source", ProofDisposition::Passed);
        let accepted = decide_promise(&promise, &authority, &exact, None, &mut 0).unwrap();
        assert_eq!(
            accepted,
            decide_promise(&promise, &authority, &exact, None, &mut 0).unwrap()
        );
        assert_eq!(
            accept_all(&[promise], &[accepted], &ParentAcceptance::default())
                .unwrap()
                .len(),
            1
        );
    }
