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
