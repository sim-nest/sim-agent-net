#[cfg(test)]
#[allow(dead_code)]
#[path = "../../src/refiner.rs"]
mod refiner_product;

#[cfg(test)]
mod tests {
    use sim_kernel::{ContentId, Lib, Symbol};
    use sim_lib_journal::{
        Admission, JournalBackend, JournalError, JournalHead, Lease, MemoryBackend, StoredState,
    };
    use sim_lib_roadmap_runner::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn proof_content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }

    fn local_request(verb: &str, observe: bool) -> LocalRoadmapRequest {
        LocalRoadmapRequest {
            verb: verb.into(),
            observe,
            disposable_checkout: None,
            local_authority_token: None,
            identity: LocalExecutionIdentity {
                execution: "local/specimen".into(),
                conduct: "sha256:conduct".into(),
                model_pick: "sha256:no-model".into(),
                proof_catalog: "sha256:proofs".into(),
                runner_generation: "sha256:generation".into(),
            },
        }
    }

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
            accept_all(&[promise.clone()], &[refuted], &ParentAcceptance::default()),
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

    fn pins(n: u8) -> ExecutionPins {
        ExecutionPins {
            conduct: format!("conduct-{n}"),
            policy: format!("policy-{n}"),
            source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [n; 32]),
            model_pick: format!("model-{n}"),
            runner_generation: format!("runner-{n}"),
        }
    }

    #[test]
    fn receipt_only_replay_is_exact_and_performs_no_effects() {
        let log = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "exec", Limits::default());
        let mut state = log.open(pins(1), None).unwrap();
        let packet = log
            .prepare_object(
                ObjectKind::Packet,
                b"bounded packet".to_vec(),
                "packet summary",
            )
            .unwrap();
        for (record, objects) in [
            (
                ExecutionRecord::StateTransition {
                    from: "planned".into(),
                    to: "running".into(),
                },
                vec![],
            ),
            (
                ExecutionRecord::EffectRequested {
                    effect_id: "effect-1".into(),
                    kind: "process".into(),
                    input: Some(packet.reference.clone()),
                },
                vec![packet],
            ),
        ] {
            state.head = log.append(Some(&state.head), record, objects).unwrap();
        }
        let output = log
            .prepare_object(ObjectKind::ProcessOutput, b"ok".to_vec(), "exit zero")
            .unwrap();
        state.head = log
            .append(
                Some(&state.head),
                ExecutionRecord::EffectReceipt {
                    effect_id: "effect-1".into(),
                    outcome: "ok".into(),
                    output: Some(output.reference.clone()),
                },
                vec![output],
            )
            .unwrap();
        for record in [
            ExecutionRecord::MutationFence {
                mutation_id: "m1".into(),
                expected: "preimage".into(),
            },
            ExecutionRecord::ProofResult {
                proof: "tests".into(),
                passed: true,
                evidence: None,
            },
            ExecutionRecord::Discharge {
                obligation: "tests".into(),
            },
            ExecutionRecord::Ambiguity {
                reason: "none".into(),
            },
            ExecutionRecord::StateTransition {
                from: "running".into(),
                to: "reconciling".into(),
            },
            ExecutionRecord::StateTransition {
                from: "reconciling".into(),
                to: "succeeded".into(),
            },
            ExecutionRecord::TerminalReceipt {
                outcome: "succeeded".into(),
            },
        ] {
            state.head = log.append(Some(&state.head), record, vec![]).unwrap();
        }
        assert_eq!(log.rebuild().unwrap(), log.rebuild().unwrap());
        assert_eq!(log.rebuild().unwrap().records.len(), 11);
    }

    #[test]
    fn stale_writers_duplicate_receipts_secrets_limits_and_pin_drift_fail_closed() {
        let log = ExecutionJournal::new(
            Arc::new(MemoryBackend::new()),
            "exec",
            Limits {
                max_object_bytes: 64,
                ..Limits::default()
            },
        );
        let opened = log.open(pins(1), None).unwrap();
        for bytes in [
            b"API_KEY=abc".as_slice(),
            b"Authorization: Bearer abc".as_slice(),
            b"-----PRIVATE KEY-----".as_slice(),
        ] {
            assert!(matches!(
                log.prepare_object(ObjectKind::Packet, bytes.to_vec(), "safe"),
                Err(ExecutionJournalError::Secret)
            ));
        }
        assert!(matches!(
            log.prepare_object(ObjectKind::FileBytes, vec![0; 65], "safe"),
            Err(ExecutionJournalError::Budget("object"))
        ));
        let head = log
            .append(
                Some(&opened.head),
                ExecutionRecord::EffectRequested {
                    effect_id: "x".into(),
                    kind: "write".into(),
                    input: None,
                },
                vec![],
            )
            .unwrap();
        assert!(matches!(
            log.append(
                Some(&opened.head),
                ExecutionRecord::Ambiguity {
                    reason: "stale".into()
                },
                vec![]
            ),
            Err(ExecutionJournalError::Journal(
                JournalError::WrongHead | JournalError::ConflictingDelivery
            ))
        ));
        let receipt = ExecutionRecord::EffectReceipt {
            effect_id: "x".into(),
            outcome: "ok".into(),
            output: None,
        };
        let head = log.append(Some(&head), receipt.clone(), vec![]).unwrap();
        assert!(matches!(
            log.append(Some(&head), receipt, vec![]),
            Err(ExecutionJournalError::Illegal { .. })
        ));
        assert!(matches!(
            log.open(pins(2), Some(&head)),
            Err(ExecutionJournalError::ChildRequired { .. })
        ));
    }

    struct InspectBackend {
        inner: MemoryBackend,
        crash: AtomicBool,
    }
    impl Default for InspectBackend {
        fn default() -> Self {
            Self {
                inner: MemoryBackend::new(),
                crash: AtomicBool::new(false),
            }
        }
    }
    impl JournalBackend for InspectBackend {
        fn acquire_lease(&self) -> Result<Lease, JournalError> {
            self.inner.acquire_lease()
        }
        fn read_state(&self) -> Result<StoredState, JournalError> {
            self.inner.read_state()
        }
        fn admit(&self, a: Admission) -> Result<JournalHead, JournalError> {
            if self.crash.swap(false, Ordering::SeqCst) {
                Err(JournalError::InjectedCrash("adapter-admission"))
            } else {
                self.inner.admit(a)
            }
        }
    }
    struct SnapshotBackend(StoredState);
    impl JournalBackend for SnapshotBackend {
        fn acquire_lease(&self) -> Result<Lease, JournalError> {
            Err(JournalError::WriteRefused("snapshot"))
        }
        fn read_state(&self) -> Result<StoredState, JournalError> {
            Ok(self.0.clone())
        }
        fn admit(&self, _: Admission) -> Result<JournalHead, JournalError> {
            Err(JournalError::WriteRefused("snapshot"))
        }
    }

    #[test]
    fn crash_missing_object_truncated_reordered_and_cross_run_history_report_last_verified_head() {
        let backend = Arc::new(InspectBackend::default());
        let log = ExecutionJournal::new(backend.clone(), "exec", Limits::default());
        let opened = log.open(pins(1), None).unwrap();
        let object = log
            .prepare_object(ObjectKind::FileBytes, b"bytes".to_vec(), "file")
            .unwrap();
        backend.crash.store(true, Ordering::SeqCst);
        assert!(matches!(
            log.append(
                Some(&opened.head),
                ExecutionRecord::EffectRequested {
                    effect_id: "write".into(),
                    kind: "file".into(),
                    input: Some(object.reference.clone())
                },
                vec![object]
            ),
            Err(ExecutionJournalError::Journal(JournalError::InjectedCrash(
                _
            )))
        ));
        assert_eq!(log.rebuild().unwrap().head, opened.head);
        let head = log
            .append(
                Some(&opened.head),
                ExecutionRecord::Ambiguity {
                    reason: "recover".into(),
                },
                vec![],
            )
            .unwrap();
        let state = backend.read_state().unwrap();
        let mut missing = state.clone();
        missing.objects.clear();
        let report = ExecutionJournal::new(
            Arc::new(SnapshotBackend(missing)),
            "exec",
            Limits::default(),
        )
        .rebuild_report()
        .unwrap_err();
        assert!(report.last_verified_head.is_none());
        let mut truncated = state.clone();
        truncated.entries.remove(&head.sequence);
        let report = ExecutionJournal::new(
            Arc::new(SnapshotBackend(truncated)),
            "exec",
            Limits::default(),
        )
        .rebuild_report()
        .unwrap_err();
        assert_eq!(report.last_verified_head, Some(opened.head.clone()));
        let mut reordered = state.clone();
        reordered.entries.get_mut(&1).unwrap().sequence = 2;
        let report = ExecutionJournal::new(
            Arc::new(SnapshotBackend(reordered)),
            "exec",
            Limits::default(),
        )
        .rebuild_report()
        .unwrap_err();
        assert_eq!(report.last_verified_head, Some(opened.head));
        assert!(matches!(
            ExecutionJournal::new(
                Arc::new(SnapshotBackend(state)),
                "foreign-exec",
                Limits::default()
            )
            .rebuild_report()
            .unwrap_err()
            .error,
            ExecutionJournalError::ExecutionIdentity
        ));
    }
}

#[cfg(test)]
mod implementer_contract {
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
}

#[cfg(test)]
mod supervisor_service {
    use std::sync::{Arc, Mutex};
    use sim_kernel::{ContentId, Symbol};
    use sim_lib_journal::MemoryBackend;
    use sim_lib_roadmap_runner::*;
    use sim_roadmap_core::PhaseId;
    use sim_roadmap_exec_core::*;

    fn id(n: u8) -> ContentId { ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [n; 32]) }
    struct Ready(PhaseId);
    impl ReadinessPort for Ready { fn admitted_leaf(&self, _: &ContentId) -> Result<Option<PhaseId>, ServiceError> { Ok(Some(self.0.clone())) } }
    struct Effects(Mutex<Vec<ExecutionEvent>>);
    impl EffectPort for Effects {
        fn invoke(&self, identity: &ExecutionIdentity, phase: &PhaseId, _: &Transition) -> Result<ExecutionEvent, ServiceError> {
            let event = ExecutionEvent { execution: identity.execution.clone(), phase: phase.clone(), attempt: AttemptId::new("a").unwrap(), observation: Observation { kind: Symbol::new("start"), journal_head: id(20), ..Default::default() } };
            self.0.lock().unwrap().push(event.clone()); Ok(event)
        }
        fn receipts(&self, _: &ExecutionIdentity) -> Result<Vec<ExecutionEvent>, ServiceError> { Ok(self.0.lock().unwrap().clone()) }
    }
    struct Cancel(bool);
    impl CancellationPort for Cancel { fn cancellation_requested(&self, _: &ExecutionId) -> bool { self.0 } }
    fn request() -> OpenRequest {
        let policy = ExecutionPolicy { id: ExecutionPolicyId::new("p").unwrap(), source_deck: id(2), required_promises: vec![], required_proofs: vec![] };
        OpenRequest { authority: AuthorityGrant { identity: ExecutionIdentity { execution: ExecutionId::new("e").unwrap(), policy: policy.id.clone(), roadmap: id(1), source_deck: id(2), conduct: id(3), model: id(4), launcher: id(5), runner: id(6) }, ceiling: EffectiveCeiling::intersect([OwnedLimit { owner: Symbol::new("caller"), unit: Symbol::new("turns"), amount: 1 }]), grant: id(7) }, phase: PhaseId::new("leaf").unwrap(), attempt: AttemptId::new("a").unwrap(), policy, mutation: MutationPlan::new(MutationId::new("m").unwrap(), vec![], vec![]).unwrap() }
    }
    #[test]
    fn fake_and_service_boundaries_replay_identically_and_reject_identity_drift() {
        let request = request();
        let mut service = RoadmapRunnerService::open(Arc::new(MemoryBackend::new()), Ready(request.phase.clone()), Effects(Mutex::new(vec![])), Cancel(false), request.clone(), Limits::default()).unwrap();
        let advanced = service.advance_one_effect().unwrap();
        assert_eq!(advanced.transition, service.replay().unwrap());
        assert!(matches!(advanced.journal.records[1], ExecutionRecord::EffectRequested { .. }));
        let mut replacement = request.authority; replacement.identity.model = id(99); replacement.grant = id(8);
        assert!(matches!(service.resume(replacement), Err(ServiceError::IdentityDrift)));
    }
}

#[cfg(test)]
mod mutation_contract {
    use std::collections::BTreeMap;
    use sim_lib_roadmap_runner::*;

    #[derive(Clone)]
    struct MemoryWorkspace {
        files: BTreeMap<String, PortableImage>,
        scratch: Option<u64>,
        durability: Durability,
    }
    impl MemoryWorkspace {
        fn with(files: impl IntoIterator<Item = (&'static str, PortableImage)>) -> Self {
            Self { files: files.into_iter().map(|(p, i)| (p.into(), i)).collect(), scratch: None, durability: Durability::FileAndDirectorySync }
        }
    }
    impl MutationWorkspace for MemoryWorkspace {
        fn observe(&self, path: &str) -> Result<PortableImage, MutationError> {
            Ok(self.files.get(path).cloned().unwrap_or_else(PortableImage::absent))
        }
        fn scratch_available(&self) -> Result<Option<u64>, MutationError> { Ok(self.scratch) }
        fn apply(&mut self, entry: &SealedEntry, fail: &mut dyn FnMut(Failpoint) -> Result<(), MutationError>) -> Result<(), MutationError> {
            for point in [Failpoint::BeforeTempWrite, Failpoint::AfterTempWrite, Failpoint::BeforeFlush, Failpoint::AfterFlush, Failpoint::BeforeReplace] { fail(point)?; }
            match &entry.postimage.bytes {
                Some(_) => { self.files.insert(entry.path.clone(), entry.postimage.clone()); }
                None => { self.files.remove(&entry.path); }
            }
            for point in [Failpoint::AfterReplace, Failpoint::BeforeDirectorySync, Failpoint::AfterDirectorySync] { fail(point)?; }
            Ok(())
        }
        fn durability(&self) -> Durability { self.durability }
    }
    #[derive(Default)]
    struct Journal { plans: Vec<[u8; 32]>, fences: Vec<MutationFence> }
    impl MutationJournal for Journal {
        fn put_plan(&mut self, plan: &SealedMutationPlan) -> Result<(), MutationError> { self.plans.push(plan.id); Ok(()) }
        fn append_fence(&mut self, _: [u8; 32], fence: MutationFence) -> Result<(), MutationError> { self.fences.push(fence); Ok(()) }
    }
    fn image(bytes: impl Into<Vec<u8>>, mode: u32) -> PortableImage { PortableImage::file(bytes, mode) }
    fn plan() -> SealedMutationPlan {
        SealedMutationPlan::seal(vec![
            StructuralEdit { path: "z-delete".into(), preimage: image(b"old".to_vec(), 0o644), postimage: PortableImage::absent() },
            StructuralEdit { path: "a-create".into(), preimage: PortableImage::absent(), postimage: image(Vec::new(), 0o600) },
            StructuralEdit { path: "m-edit".into(), preimage: image(vec![0xff, 0], 0o755), postimage: image(vec![0xfe, 1], 0o755) },
        ]).unwrap()
    }

    #[test]
    fn sealing_is_sorted_unique_exact_and_rejects_collisions() {
        let p = plan();
        assert_eq!(p.entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["a-create", "m-edit", "z-delete"]);
        let reversed = SealedMutationPlan::seal(p.entries.iter().rev().map(|e| StructuralEdit { path: e.path.clone(), preimage: e.preimage.clone(), postimage: e.postimage.clone() }).collect()).unwrap();
        assert_eq!(p.id, reversed.id);
        let duplicate = vec![StructuralEdit { path: "a".into(), preimage: PortableImage::absent(), postimage: image(b"1".to_vec(), 0o644) }; 2];
        assert!(matches!(SealedMutationPlan::seal(duplicate), Err(MutationError::DuplicatePath)));
        let collision = vec![
            StructuralEdit { path: "a".into(), preimage: PortableImage::absent(), postimage: image(b"1".to_vec(), 0o644) },
            StructuralEdit { path: "a/b".into(), preimage: PortableImage::absent(), postimage: image(b"2".to_vec(), 0o644) },
        ];
        assert!(matches!(SealedMutationPlan::seal(collision), Err(MutationError::ParentChildCollision)));
    }

    #[test]
    fn preflight_rejects_foreign_images_capacity_paths_and_modes_before_journaling() {
        let p = plan();
        let mut ws = MemoryWorkspace::with([("m-edit", image(b"foreign".to_vec(), 0o755)), ("z-delete", image(b"old".to_vec(), 0o644))]);
        ws.scratch = Some(0);
        let mut engine = MutationEngine { workspace: ws, journal: Journal::default() };
        assert!(matches!(engine.execute(&p), Err(MutationError::PreimageMismatch(_))));
        assert!(engine.journal.plans.is_empty());
        let bad_path = StructuralEdit { path: "../escape".into(), preimage: PortableImage::absent(), postimage: image(b"x".to_vec(), 0o644) };
        assert!(matches!(SealedMutationPlan::seal(vec![bad_path]), Err(MutationError::InvalidPath)));
        let bad_mode = StructuralEdit { path: "x".into(), preimage: PortableImage::absent(), postimage: image(b"x".to_vec(), 0o10644) };
        assert!(matches!(SealedMutationPlan::seal(vec![bad_mode]), Err(MutationError::InvalidMode)));
    }

    #[test]
    fn create_edit_delete_empty_non_utf8_and_modes_commit() {
        let p = plan();
        let ws = MemoryWorkspace::with([("m-edit", image(vec![0xff, 0], 0o755)), ("z-delete", image(b"old".to_vec(), 0o644))]);
        let mut engine = MutationEngine { workspace: ws, journal: Journal::default() };
        let receipt = engine.execute(&p).unwrap();
        assert_eq!(receipt.plan_id, p.id);
        assert_eq!(engine.workspace.observe("a-create").unwrap(), image(Vec::new(), 0o600));
        assert_eq!(engine.workspace.observe("m-edit").unwrap(), image(vec![0xfe, 1], 0o755));
        assert_eq!(engine.workspace.observe("z-delete").unwrap(), PortableImage::absent());
        assert_eq!(engine.journal.fences.last(), Some(&MutationFence::Committed));
    }

    #[test]
    fn every_failpoint_reopens_to_commit_or_safe_ambiguity_without_losing_images() {
        let points = [
            Failpoint::BeforeObjectPut, Failpoint::AfterObjectPut, Failpoint::BeforeJournalAppend, Failpoint::AfterJournalAppend,
            Failpoint::BeforeTempWrite, Failpoint::AfterTempWrite, Failpoint::BeforeFlush, Failpoint::AfterFlush,
            Failpoint::BeforeReplace, Failpoint::AfterReplace, Failpoint::BeforeDirectorySync, Failpoint::AfterDirectorySync,
            Failpoint::BeforePostimageObservation, Failpoint::AfterPostimageObservation,
        ];
        for target in points {
            let p = plan();
            let ws = MemoryWorkspace::with([("m-edit", image(vec![0xff, 0], 0o755)), ("z-delete", image(b"old".to_vec(), 0o644))]);
            let mut engine = MutationEngine { workspace: ws, journal: Journal::default() };
            let mut fired = false;
            let result = engine.execute_with(&p, |point| if point == target && !fired { fired = true; Err(MutationError::Injected(point)) } else { Ok(()) });
            assert!(result.is_err(), "failpoint {target:?} was not reached");
            engine.resume(&p).unwrap();
            assert!(matches!(classify_plan(&p, &engine.observe_all(&p).unwrap()), ResumeDecision::Committed));
            assert!(p.entries.iter().all(|e| e.preimage.bytes.is_some() || e.postimage.bytes.is_some()));
        }
    }

    #[test]
    fn concurrent_edit_and_symlink_are_ambiguous_and_inverse_is_exact() {
        let p = plan();
        let mut committed = BTreeMap::new();
        for e in &p.entries { committed.insert(e.path.clone(), e.postimage.clone()); }
        let inverse = inverse_plan(&p, &committed).unwrap();
        assert!(inverse.entries.iter().zip(&p.entries).all(|(i, e)| i.preimage == e.postimage && i.postimage == e.preimage));
        committed.insert("m-edit".into(), image(b"user".to_vec(), 0o755));
        assert!(matches!(inverse_plan(&p, &committed), Err(MutationError::Ambiguous { .. })));

        let root = std::env::temp_dir().join(format!("sim-mutation-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        #[cfg(unix)] {
            std::os::unix::fs::symlink("elsewhere", root.join("m-edit")).unwrap();
            let fs = FsWorkspace::new(&root).unwrap();
            assert!(matches!(fs.observe("m-edit"), Err(MutationError::UnsupportedFileKind(_))));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn weak_adapter_reports_only_the_durability_it_proves_and_disk_full_is_preflighted() {
        let p = plan();
        let mut ws = MemoryWorkspace::with([("m-edit", image(vec![0xff, 0], 0o755)), ("z-delete", image(b"old".to_vec(), 0o644))]);
        ws.durability = Durability::Replace;
        ws.scratch = Some(1);
        let mut engine = MutationEngine { workspace: ws, journal: Journal::default() };
        assert!(matches!(engine.execute(&p), Err(MutationError::ScratchCapacity)));
        engine.workspace.scratch = Some(100);
        assert_eq!(engine.execute(&p).unwrap().durability, Durability::Replace);
    }

    #[test]
    fn generic_execution_journal_durably_carries_plan_object_and_fences() {
        use sim_kernel::{ContentId, Symbol};
        use sim_lib_journal::MemoryBackend;
        use std::sync::Arc;

        let journal = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "mutation", Limits::default());
        journal.open(ExecutionPins {
            conduct: "conduct".into(), policy: "policy".into(),
            source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [7; 32]),
            model_pick: "none".into(), runner_generation: "runner".into(),
        }, None).unwrap();
        let p = plan();
        let ws = MemoryWorkspace::with([("m-edit", image(vec![0xff, 0], 0o755)), ("z-delete", image(b"old".to_vec(), 0o644))]);
        let mut engine = MutationEngine { workspace: ws, journal };
        engine.execute(&p).unwrap();
        let rebuilt = engine.journal.rebuild().unwrap();
        assert!(matches!(rebuilt.records.get(1), Some(ExecutionRecord::EffectRequested { kind, input: Some(_), .. }) if kind == "sealed-mutation-plan"));
        assert!(matches!(rebuilt.records.last(), Some(ExecutionRecord::MutationFence { expected, .. }) if expected == "committed"));
    }
}

#[cfg(test)]
mod proof_leaf_tests {
    use sim_lib_exec::*;
    use sim_lib_journal::MemoryBackend;
    use sim_lib_roadmap_runner::*;
    use sim_kernel::{ContentId, Symbol};
    use std::{collections::{BTreeMap, BTreeSet}, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}};

    fn limits() -> SandboxLimits { SandboxLimits { cpu_seconds: 1, memory_bytes: 1_000_000,
        wall_time_ms: 50, process_count: 2, file_count: 2, file_bytes: 100, output_bytes: 100, stdin_bytes: 1 } }
    fn command(name: &str) -> CommandProof { CommandProof { name: name.into(), effect_id: format!("effect-{name}"),
        program: "tool:harmless".into(), argv: vec!["literal;not-shell".into()], working_directory: "/source".into(),
        environment: BTreeMap::new(), allowed_environment_keys: BTreeSet::new(), source_mount: "source:deck".into(),
        scratch_mount: Some("scratch:proof".into()), source_read_only: true, limits: limits(),
        expected: StructuredExpectation { stdout_sha256: "2689367b205c16ce32ed4200942b8b8b1e262dfc70d9bc9fbc77c49699a4f1df".into(), exit_code: 0 } } }
    fn report(request: &SandboxRequest, hits: Vec<String>) -> SandboxReport { SandboxReport {
        launcher: "fixture".into(), controls: request.policy.requirements().keys().map(|control| SandboxEvidence {
            control: *control, achieved: true, detail: "isolated".into() }).collect(), limit_hits: hits, cleanup: "sandbox:bounded/reaped".into() } }
    struct Launcher { calls: AtomicUsize, mode: &'static str }
    impl SandboxLauncher for Launcher {
        fn id(&self) -> &str { "fixture" }
        fn launch(&self, request: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.environment.iter().count(), 1);
            assert_eq!(request.policy.mounts()[0].access, MountAccess::ReadOnly);
            assert_eq!(request.policy.requirements()[&SandboxControl::Network], SandboxRequirement::Required);
            match self.mode {
                "ok" => SandboxAttempt::Completed(SandboxResult { stdout: b"ok".to_vec(), stderr: vec![], exit_code: 0, report: report(request, vec![]) }),
                "flood" => SandboxAttempt::Completed(SandboxResult { stdout: vec![b'x'; 100], stderr: vec![], exit_code: 0, report: report(request, vec!["output".into()]) }),
                "fork" => SandboxAttempt::Stopped(report(request, vec!["process-count".into()])),
                "timeout" => SandboxAttempt::Stopped(report(request, vec!["wall-time".into()])),
                "write" | "network" => SandboxAttempt::Refused(SandboxRefusal { launcher: "fixture".into(), reason: self.mode.into(), report: Some(report(request, vec![])) }),
                _ => SandboxAttempt::Completed(SandboxResult { stdout: b"malformed".to_vec(), stderr: vec![], exit_code: 0, report: report(request, vec![]) }),
            }
        }
    }
    struct Panic;
    impl SandboxLauncher for Panic { fn id(&self) -> &str { "panic" } fn launch(&self, _: &SandboxRequest, _: &ProcessCancellation) -> SandboxAttempt { panic!("effect during replay") } }
    #[derive(Default)] struct Store(Mutex<BTreeMap<String, TypedProofReceipt>>);
    impl ProofReceiptStore for Store { fn inspect(&self, id: &str) -> Option<TypedProofReceipt> { self.0.lock().unwrap().get(id).cloned() }
        fn record(&self, id: &str, receipt: &TypedProofReceipt) { self.0.lock().unwrap().insert(id.into(), receipt.clone()); } }

    #[test]
    fn hostile_catalog_and_sandbox_are_fail_closed() {
        let mut invalid = command("shell"); invalid.program = "sh -c env".into();
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("ambient"); invalid.environment.insert("HOME".into(), "/host".into());
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("secret"); invalid.environment.insert("API_TOKEN".into(), "steal".into()); invalid.allowed_environment_keys.insert("API_TOKEN".into());
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("absolute"); invalid.source_mount = "/host/source".into();
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        let mut invalid = command("writeable"); invalid.source_read_only = false;
        assert!(ProofCatalog::new([ProofLeaf::Command(invalid)]).is_err());
        for mode in ["flood", "fork", "timeout", "malformed", "write", "network"] {
            let catalog = ProofCatalog::new([ProofLeaf::Command(command(mode))]).unwrap();
            let receipt = execute_proof_leaf(&catalog, mode, &Launcher { calls: AtomicUsize::new(0), mode }, &ProcessCancellation::default(), "fixed").unwrap();
            assert_ne!(receipt.disposition, ProofDisposition::Passed, "{mode}");
        }
    }

    #[test]
    fn pure_deck_and_artifact_proofs_never_launch() {
        let catalog = ProofCatalog::new([ProofLeaf::ArtifactEquality { name: "artifact".into(), left: b"x".to_vec(), right: b"x".to_vec() },
            ProofLeaf::SourceDeckPredicate { name: "deck".into(), actual_deck: "deck:1".into(), expected_deck: "deck:1".into(),
                required_claims: BTreeSet::from(["claim".into()]), present_claims: BTreeSet::from(["claim".into()]) }]).unwrap();
        for name in ["artifact", "deck"] { assert_eq!(execute_proof_leaf(&catalog, name, &Panic, &ProcessCancellation::default(), "fixed").unwrap().disposition, ProofDisposition::Passed); }
    }

    #[test]
    fn recorded_no_model_command_replays_without_launch() {
        let catalog = ProofCatalog::new([ProofLeaf::Command(command("no-model"))]).unwrap();
        let journal = ExecutionJournal::new(Arc::new(MemoryBackend::new()), "proof", Limits::default());
        let pins = ExecutionPins { conduct: "conduct".into(), policy: "policy".into(), source_deck: ContentId::from_bytes(Symbol::qualified("deck", "sha256-v1"), [1; 32]), model_pick: "none".into(), runner_generation: "runner".into() };
        let mut state = journal.open(pins, None).unwrap(); let store = Store::default();
        let launcher = Launcher { calls: AtomicUsize::new(0), mode: "ok" };
        let first = execute_journaled_proof(&journal, &mut state, &catalog, "no-model", &launcher, &store, &ProcessCancellation::default(), "fixed").unwrap();
        assert_eq!(first.disposition, ProofDisposition::Passed); assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
        let second = execute_journaled_proof(&journal, &mut journal.rebuild().unwrap(), &catalog, "no-model", &Panic, &store, &ProcessCancellation::default(), "ignored").unwrap();
        assert_eq!(first, second);
    }

    fn recovery_content(byte: u8) -> ContentId {
        ContentId::from_bytes(Symbol::qualified("core", "sha256-datum-v1"), [byte; 32])
    }

    #[test]
    fn recovery_reconciles_foreign_bytes_and_certifies_strict_descent() {
        let policy = sim_roadmap_exec_core::RecoveryPolicy {
            max_refinement_rank: 4,
            ..Default::default()
        };
        for n in 0..512 {
            let ambiguous = ResumeDecision::Ambiguous {
                foreign_paths: vec![format!("foreign-{n}.rs")],
            };
            let effect = plan_refinement_after_reconciliation(
                &ambiguous,
                true,
                recovery_content((n % 250 + 1) as u8),
                3,
                2,
                &policy,
            );
            assert!(matches!(
                effect,
                RecoveryEffect::Stop(RecoveryStop::Ambiguous { .. })
            ));
            assert!(terminal_requests_no_effects(&effect));
        }
        let profile = recovery_content(7);
        assert_eq!(
            plan_refinement_after_reconciliation(
                &ResumeDecision::Committed,
                true,
                profile.clone(),
                4,
                3,
                &policy,
            ),
            RecoveryEffect::InvokeRefiner {
                derived_profile: profile
            }
        );
        for (fresh, parent, child) in [(false, 4, 3), (true, 4, 4), (true, 4, 5)] {
            assert!(terminal_requests_no_effects(
                &plan_refinement_after_reconciliation(
                    &ResumeDecision::Committed,
                    fresh,
                    recovery_content(8),
                    parent,
                    child,
                    &policy,
                )
            ));
        }
    }
}
