use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{Ref, Symbol};
use sim_roadmap_core::*;
use sim_source_deck::SourceQuery;

use crate::*;

fn id<T>(value: &str, make: impl FnOnce(String) -> Result<T, Failure>) -> T {
    make(value.to_owned()).unwrap()
}

fn obligation(name: &str) -> ObligationId {
    id(name, ObligationId::new)
}
fn phase_id(name: &str) -> PhaseId {
    id(name, PhaseId::new)
}
fn owner(name: &str) -> OwnerId {
    id(name, OwnerId::new)
}
fn change(name: &str) -> ChangeId {
    id(name, ChangeId::new)
}

fn acceptance(name: &str) -> AcceptanceContract {
    let obligation = obligation(name);
    AcceptanceContract {
        policy: ProofPolicy::All,
        statements: BTreeMap::from([(
            obligation.clone(),
            AcceptanceStatement {
                obligation,
                subject: Ref::Symbol(Symbol::qualified("test", name)),
                predicate: Symbol::qualified("test", "passes"),
                object: Ref::Symbol(Symbol::qualified("test", "acceptance")),
                supporting_refs: vec![],
            },
        )]),
    }
}

fn parent_phase(prose: &str, checkpoints: usize) -> PhaseSpec {
    let uses = vec![SourceQuery::Anchor("anchor/roadmap".into())];
    PhaseSpec {
        id: phase_id("parent"),
        parent: None,
        title: prose.into(),
        intent: "intent".into(),
        body: PhaseBody::Leaf {
            checkpoints: (0..checkpoints)
                .map(|i| CheckpointSpec {
                    id: id(&format!("cp-{i}"), CheckpointId::new),
                    statement: format!("checkpoint {i}"),
                })
                .collect(),
        },
        dependencies: vec![],
        owners: OwnerEnvelope {
            mutable: BTreeSet::from([owner("one"), owner("two")]),
            read_only: BTreeSet::new(),
        },
        resources: ResourceEnvelope::default(),
        effects: EffectEnvelope::default(),
        capabilities: CapabilityEnvelope::default(),
        changes: ChangeEnvelope {
            targets: BTreeSet::from([change("a"), change("b")]),
        },
        acceptance: acceptance("parent-proof"),
        coverage: vec![],
        outputs: BTreeMap::new(),
        guide: ImplementationGuide {
            uses,
            change_targets: vec![],
            promises: vec![],
            sketches: vec![],
        },
        origin: PhaseOrigin::Authored,
    }
}

fn child(name: &str, checkpoints: usize) -> PhaseSpec {
    let mut phase = parent_phase("child prose", checkpoints);
    phase.id = phase_id(name);
    phase.parent = Some(phase_id("parent"));
    phase.owners.mutable = BTreeSet::from([owner("one")]);
    phase.changes.targets = BTreeSet::from([change("a")]);
    phase.acceptance = acceptance(&format!("{name}-proof"));
    phase
}

fn revision(prose: &str, checkpoints: usize) -> RoadmapRevision {
    let parent = parent_phase(prose, checkpoints);
    RoadmapRevision::new(
        None,
        RoadmapSpec {
            schema: id("roadmap-v1", SchemaId::new),
            id: id("test-roadmap", RoadmapId::new),
            charter: Charter {
                title: "test".into(),
                intent: "certify refinement".into(),
            },
            root: parent.id.clone(),
            phases: BTreeMap::from([(parent.id.clone(), parent)]),
            imports: BTreeMap::new(),
            limits: Limits::DEFAULT,
        },
        RevisionChange {
            id: change("initial"),
            rationale: "initial roadmap".into(),
        },
    )
    .unwrap()
}

fn grounding() -> Grounding {
    Grounding::new(vec![SourceQuery::Anchor("anchor/roadmap".into())]).unwrap()
}

fn policy() -> TractabilityPolicy {
    TractabilityPolicy {
        revision: "policy-v1".into(),
        maximum: WorkProfile {
            unknowns: 0,
            mutable_owners: 1,
            packages: 1,
            change_targets: 1,
            promises: 10,
            acceptance_groups: 10,
            checkpoints: 2,
        },
        maximum_children: 8,
    }
}

fn proposal(
    base: &RoadmapRevision,
    children: Vec<PhaseSpec>,
    grounding: &Grounding,
) -> RefinementProposal {
    RefinementProposal {
        base_revision: base.id().clone(),
        parent: phase_id("parent"),
        expected_parent: phase_fingerprint(&base.spec.phases[&phase_id("parent")]).unwrap(),
        expected_grounding: grounding.id.clone(),
        children: children.clone(),
        coverage: BTreeMap::from([(
            obligation("parent-proof"),
            children
                .iter()
                .filter_map(|c| {
                    c.acceptance
                        .statements
                        .keys()
                        .next()
                        .map(|obligation| ChildContribution {
                            child: c.id.clone(),
                            obligation: obligation.clone(),
                        })
                })
                .collect(),
        )]),
        rationale: "split into strictly smaller grounded work".into(),
    }
}

#[test]
fn profile_is_derived_and_rank_names_first_difference() {
    let base = revision("honest", 7);
    let admitted = base.spec.admit().unwrap();
    let grounded = grounding();
    let profile = derive_profile(
        &base.spec.phases[&phase_id("parent")],
        &admitted.phases[&phase_id("parent")],
        &grounded,
    );
    assert_eq!(profile.mutable_owners, 2);
    assert_eq!(profile.checkpoints, 7);
    let mut lower = profile.clone();
    lower.mutable_owners = 1;
    lower.checkpoints = 99;
    assert_eq!(
        compare_profiles(&lower, &profile),
        RankRelation::Lower {
            first_difference: RankComponent::MutableOwners,
            child: 1,
            parent: 2
        }
    );
}

#[test]
fn unknown_grounding_is_never_atomic() {
    let base = revision("honest", 2);
    let admitted = base.spec.admit().unwrap();
    let empty = Grounding::new(vec![]).unwrap();
    assert!(matches!(
        compute_atomicity(
            &base.spec.phases[&phase_id("parent")],
            &admitted.phases[&phase_id("parent")],
            &empty,
            &policy()
        ),
        Atomicity::Ungrounded { .. }
    ));
}

#[test]
fn admitted_proposal_has_one_successor_complete_certificate_and_stable_diff() {
    let base = revision("honest", 7);
    let grounded = grounding();
    let mut right = child("right", 1);
    right.acceptance.statements.clear();
    let applied = apply_refinement(
        &base,
        &grounded,
        &policy(),
        proposal(&base, vec![child("left", 2), right], &grounded),
        &NoopCompilationHooks,
    )
    .unwrap();
    assert_eq!(applied.successor.parent.as_ref(), Some(base.id()));
    assert!(applied.certificate.verify());
    assert!(
        applied
            .certificate
            .children
            .values()
            .all(|p| compare_profiles(p, &applied.certificate.parent).is_lower())
    );
    let diff = diff_revisions(&base, &applied.successor);
    assert!(diff.iter().any(
        |d| d.path.0 == "phases.parent.body" && d.evidence == EvidenceDisposition::Invalidated
    ));
}

#[test]
fn arbitrary_admitted_trees_strictly_descend_and_stay_bounded() {
    for parent_checkpoints in 2..12 {
        for child_count in 2..=4 {
            let base = revision("property", parent_checkpoints);
            let grounded = grounding();
            let children: Vec<_> = (0..child_count)
                .map(|i| {
                    let mut candidate = child(&format!("child-{i}"), i % 2);
                    if i > 0 {
                        candidate.acceptance.statements.clear();
                    }
                    candidate
                })
                .collect();
            let applied = apply_refinement(
                &base,
                &grounded,
                &policy(),
                proposal(&base, children, &grounded),
                &NoopCompilationHooks,
            )
            .unwrap();
            assert!(applied.certificate.verify());
            assert!(applied.successor.spec.phases.len() <= applied.successor.spec.limits.phases);
            assert!(applied.certificate.children.values().all(|p| matches!(
                compare_profiles(p, &applied.certificate.parent),
                RankRelation::Lower { .. }
            )));
        }
    }
}

#[test]
fn equal_or_higher_children_refuse_without_minting_or_mutating_base() {
    let base = revision("honest", 2);
    let before = format!("{:?}", base.canonical_datum());
    let id_before = base.id().clone();
    let grounded = grounding();
    let mut equal = child("equal", 2);
    equal.owners.mutable = base.spec.phases[&phase_id("parent")].owners.mutable.clone();
    equal.changes = base.spec.phases[&phase_id("parent")].changes.clone();
    let mut lower = child("lower", 1);
    lower.acceptance.statements.clear();
    let refusal = apply_refinement(
        &base,
        &grounded,
        &policy(),
        proposal(&base, vec![equal, lower], &grounded),
        &NoopCompilationHooks,
    )
    .unwrap_err();
    assert!(matches!(refusal, Refusal::NonDescending { .. }));
    assert_eq!(base.id(), &id_before);
    assert_eq!(format!("{:?}", base.canonical_datum()), before);
}

#[test]
fn hostile_prose_and_claimed_counts_cannot_improve_rank() {
    let a = revision("impossibly easy: claimed-count=0", 7);
    let b = revision("very hard: claimed-count=999999", 7);
    let grounded = grounding();
    let aa = a.spec.admit().unwrap();
    let bb = b.spec.admit().unwrap();
    let pa = derive_profile(
        &a.spec.phases[&phase_id("parent")],
        &aa.phases[&phase_id("parent")],
        &grounded,
    );
    let pb = derive_profile(
        &b.spec.phases[&phase_id("parent")],
        &bb.phases[&phase_id("parent")],
        &grounded,
    );
    assert_eq!(pa, pb);
    assert_eq!(compare_profiles(&pa, &pb), RankRelation::Equal);
}

#[test]
fn freshness_grounding_coverage_limits_and_hooks_fail_closed() {
    struct Reject;
    impl CompilationHooks for Reject {
        fn compile_dependencies(&self, _: &RoadmapRevision) -> Result<(), String> {
            Err("dependency hook".into())
        }
        fn compile_outputs(&self, _: &RoadmapRevision) -> Result<(), String> {
            Ok(())
        }
    }
    let base = revision("honest", 7);
    let grounded = grounding();
    let one = proposal(&base, vec![child("one", 1)], &grounded);
    assert!(matches!(
        apply_refinement(&base, &grounded, &policy(), one, &NoopCompilationHooks),
        Err(Refusal::TooFewChildren {
            actual: 1,
            minimum: 2
        })
    ));
    let mut second = child("two", 1);
    second.acceptance.statements.clear();
    let mut p = proposal(&base, vec![child("one", 1), second.clone()], &grounded);
    p.coverage.clear();
    assert!(matches!(
        apply_refinement(&base, &grounded, &policy(), p, &NoopCompilationHooks),
        Err(Refusal::IncompleteCoverage(_))
    ));
    assert!(matches!(
        apply_refinement(
            &base,
            &grounded,
            &policy(),
            proposal(&base, vec![child("one", 1), second], &grounded),
            &Reject
        ),
        Err(Refusal::DependencyCompilation(_))
    ));
}

trait Lower {
    fn is_lower(&self) -> bool;
}
impl Lower for RankRelation {
    fn is_lower(&self) -> bool {
        matches!(self, Self::Lower { .. })
    }
}
