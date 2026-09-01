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
