use super::*;

fn proposal() -> InstrumentProposal {
    let mut capsule = fake_change_capsule().unwrap();
    capsule.rollback_notes = vec!["restore prior generation".into()];
    InstrumentProposal {
        brief: BridgeInstrumentBrief {
            goal: "turn spoken intent into a local formatter".into(),
            non_goals: vec!["no network delivery".into()],
            shapes: vec!["shape/formatter-request".into()],
            authority: vec!["adopt-local".into()],
            privacy: vec!["discard conversation".into()],
            latency: "interactive-preview/offline-build".into(),
            stop: vec!["cancel before adopt".into()],
            acceptance: vec!["clean rebuild matches frozen artifacts".into()],
        },
        recipe: FrozenInstrumentRecipe {
            id: "recipe/formatter-v2".into(),
            source_content_id: "sha256:source-v2".into(),
            builder: "forge/native-builder".into(),
            sandbox_route: "sandbox/networkless".into(),
            hotload_route: "hotload/native-generation".into(),
            pack: PackClosure {
                roots: vec!["pack/formatter".into()],
                members: vec!["pack/formatter".into(), "pack/text-core".into()],
                digest: "sha256:closure-v2".into(),
            },
            capabilities: vec!["format-local".into()],
            validation: vec!["cargo test --offline".into()],
            artifacts: vec![
                ArtifactDigest {
                    path: "generated/schema.sx".into(),
                    content_id: "sha256:schema-v2".into(),
                },
                ArtifactDigest {
                    path: "libformatter.so".into(),
                    content_id: "sha256:lib-v2".into(),
                },
            ],
        },
        capsule,
    }
}

fn prior() -> InstrumentGeneration {
    InstrumentGeneration {
        generation: "generation-1".into(),
        recipe_id: "recipe/formatter-v1".into(),
        source_content_id: "sha256:source-v1".into(),
        artifacts: vec![],
        pack_roots: vec!["pack/formatter-v1".into()],
        pack_closure_digest: "sha256:closure-v1".into(),
        registry_selection: "registry/formatter-v1".into(),
    }
}

#[test]
fn conversation_free_recipe_rebuild_adopts_every_identity_atomically() {
    let proposal = proposal();
    let mut adoption = InstrumentAdoption::new(prior());
    assert_eq!(
        adoption.preview(&proposal).unwrap().rollback_generation,
        "generation-1"
    );
    let rebuild = proposal
        .recipe
        .artifacts
        .iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>();
    let receipt = adoption
        .adopt(
            "tx-2",
            "generation-2",
            "registry/formatter-v2",
            &proposal,
            &rebuild,
        )
        .unwrap();
    assert_eq!(receipt.pack_closure_digest, "sha256:closure-v2");
    assert_eq!(
        adoption.state().installed.artifacts,
        proposal.recipe.artifacts
    );
    assert_eq!(
        adoption.state().installed.registry_selection,
        "registry/formatter-v2"
    );
    assert_eq!(adoption.state().journal, vec![receipt]);
}

#[test]
fn cancellation_and_failed_rebuild_change_nothing_then_rollback_restores_all() {
    let proposal = proposal();
    let mut adoption = InstrumentAdoption::new(prior());
    let before = adoption.state().clone();
    adoption.cancel(&proposal).unwrap();
    assert_eq!(adoption.state(), &before);
    assert_eq!(
        adoption
            .adopt("bad", "generation-2", "registry/v2", &proposal, &[])
            .unwrap_err(),
        AdoptionError::ArtifactMismatch
    );
    assert_eq!(adoption.state(), &before);
    adoption
        .adopt(
            "tx-2",
            "generation-2",
            "registry/v2",
            &proposal,
            &proposal.recipe.artifacts,
        )
        .unwrap();
    assert_eq!(adoption.rollback().unwrap(), before.installed);
}
// conformance: instrument-adoption tests prove bounded qualification and retained evidence.
