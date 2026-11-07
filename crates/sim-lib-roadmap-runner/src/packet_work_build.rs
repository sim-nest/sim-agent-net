// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Typed packet construction and receiver serialization.

use sim_conformance_packet::{
    BoundSurface, DependencyUseSet, OwnerBinding, OwnerBindingId, OwnerCommand, OwnerDisposition,
    StorageId, SurfaceKey, SurfaceStatus, SurfaceUse, SurfaceUseRole,
};
use sim_kernel::{Datum, NumberLiteral, Symbol};
use sim_work_core::{
    DescentCertificate, ForbiddenEdge, ImplementationPacket, IndexImpact, InputBudget,
    PacketBuilder, PacketDraft, PacketInputSpec, StopCondition, SurfaceEvidence,
    SurfaceEvidenceState,
};

use super::{
    CanonicalWork, CapturedWork, DECLARED_STATUS, Derived, InputPort, NAMESPACE, TARGET_ROLE,
    WorkReceiveError, WorkSource, text_id,
};

pub(super) fn build_packet(
    input: &CanonicalWork,
    port: &mut dyn InputPort,
) -> Result<ImplementationPacket, WorkReceiveError> {
    let binding = owner_binding(input)?;
    let uses = dependency_uses(input, binding.id())?;
    let evidence = evidence(input, binding.id(), uses.id())?;
    let specs = input
        .sources
        .iter()
        .map(spec)
        .collect::<Result<Vec<_>, _>>()?;
    PacketBuilder::build(
        draft(input, &binding, &uses)?,
        &binding,
        &uses,
        &evidence,
        specs,
        port,
    )
    .map_err(super::map_work)
}

fn owner_binding(input: &CanonicalWork) -> Result<OwnerBinding, WorkReceiveError> {
    let mut surfaces = Vec::with_capacity(input.sources.len() + 1);
    for source in &input.sources {
        surfaces.push(BoundSurface {
            key: surface_key(&source.surface)?,
            public_name: source.path.clone(),
            status: SurfaceStatus::Existing,
        });
    }
    surfaces.push(BoundSurface {
        key: surface_key(&input.target_surface)?,
        public_name: input.target_surface.clone(),
        status: SurfaceStatus::Planned {
            producing_phase: input.phase.clone(),
        },
    });
    OwnerBinding::new(
        input.law.clone(),
        OwnerDisposition::ExtendExisting,
        vec![input.owner_package.clone()],
        surfaces,
        vec!["sim-lib-roadmap-runner".into()],
        "runner -> work".into(),
        surface_key("route/packet")?,
        surface_key("specimen/packet")?,
        command(&input.phase, "check")?,
        command(&input.phase, "docs")?,
    )
    .map_err(|_| WorkReceiveError::Schema {
        mirrors_agree: input.mirrors_agree,
    })
}

fn dependency_uses(
    input: &CanonicalWork,
    owner: &OwnerBindingId,
) -> Result<DependencyUseSet, WorkReceiveError> {
    let mut uses = input
        .sources
        .iter()
        .map(|source| {
            Ok(SurfaceUse {
                surface: surface_key(&source.surface)?,
                role: SurfaceUseRole::ReleasedDependency,
            })
        })
        .collect::<Result<Vec<_>, WorkReceiveError>>()?;
    uses.push(SurfaceUse {
        surface: surface_key(&input.target_surface)?,
        role: SurfaceUseRole::FundedTarget,
    });
    DependencyUseSet::new(input.phase.clone(), owner.clone(), uses).map_err(|_| {
        WorkReceiveError::Schema {
            mirrors_agree: input.mirrors_agree,
        }
    })
}

fn evidence(
    input: &CanonicalWork,
    owner: &OwnerBindingId,
    uses: &sim_conformance_packet::DependencyUseSetId,
) -> Result<Vec<SurfaceEvidence>, WorkReceiveError> {
    let source_owner =
        OwnerBindingId::from_text("owner/source").map_err(|_| WorkReceiveError::Schema {
            mirrors_agree: input.mirrors_agree,
        })?;
    let mut evidence = input
        .sources
        .iter()
        .map(|source| {
            Ok(SurfaceEvidence {
                key: surface_key(&source.surface)?,
                owner: source_owner.clone(),
                state: SurfaceEvidenceState::Released {
                    dependency_uses: uses.clone(),
                },
            })
        })
        .collect::<Result<Vec<_>, WorkReceiveError>>()?;
    evidence.push(SurfaceEvidence {
        key: surface_key(&input.target_surface)?,
        owner: owner.clone(),
        state: SurfaceEvidenceState::Planned {
            producing_phase: input.phase.clone(),
        },
    });
    Ok(evidence)
}

fn draft(
    input: &CanonicalWork,
    binding: &OwnerBinding,
    uses: &DependencyUseSet,
) -> Result<PacketDraft, WorkReceiveError> {
    let schema = WorkReceiveError::Schema {
        mirrors_agree: input.mirrors_agree,
    };
    Ok(PacketDraft {
        phase: uses.phase().to_owned(),
        owner: binding.id().clone(),
        behavior: text_id(&format!("behavior/{}", input.law)).map_err(|_| schema.clone())?,
        falsifier: text_id(&format!("falsifier/{}", input.target_surface))
            .map_err(|_| schema.clone())?,
        allowed_api: vec![text_id("type/datum").map_err(|_| schema.clone())?],
        input_budget: InputBudget {
            bytes: input.budget_bytes,
            files: input.budget_files,
            tokens: input.budget_tokens,
            output_bytes: input.budget_output,
        },
        output_contract: text_id(input.schema.as_str()).map_err(|_| schema.clone())?,
        forbidden_edges: vec![
            ForbiddenEdge {
                from: input.target_surface.clone(),
                to: "effect".into(),
            },
            ForbiddenEdge {
                from: input.target_surface.clone(),
                to: "approval".into(),
            },
        ],
        tests_first: vec![
            text_id(&format!("proof/{}", input.target_surface)).map_err(|_| schema.clone())?,
        ],
        validation: binding.validation().id().clone(),
        docs: binding.docs().id().clone(),
        index_impact: IndexImpact::None,
        descent: DescentCertificate {
            measure: Symbol::qualified(NAMESPACE, "unknowns"),
            before: input.unresolved,
            after: input.narrowed_to,
        },
        stop: StopCondition {
            condition: Symbol::qualified(NAMESPACE, "gate-green"),
            max_attempts: input.max_attempts,
        },
        commit_subject: None,
    })
}

fn spec(source: &WorkSource) -> Result<PacketInputSpec, WorkReceiveError> {
    Ok(PacketInputSpec {
        surface: surface_key(&source.surface)?,
        location: StorageId::for_bytes(&source.body),
        tokens: source.body.len() as u64,
    })
}

pub(super) fn captured(
    packet: &ImplementationPacket,
    derived: &Derived,
) -> Result<CapturedWork, WorkReceiveError> {
    let serialized = receiver_bytes(packet, derived)?;
    Ok(CapturedWork {
        packet_id: hex(&packet.id().content_id().bytes),
        serialized,
        bytes: derived.bytes,
        files: derived.files,
        tokens: derived.tokens,
        paths: derived.paths.clone(),
        role: TARGET_ROLE.to_owned(),
        status: DECLARED_STATUS.to_owned(),
        resources: derived.resources.clone(),
        receiver: derived.receiver.clone(),
    })
}

fn receiver_bytes(
    packet: &ImplementationPacket,
    derived: &Derived,
) -> Result<Vec<u8>, WorkReceiveError> {
    let resources = derived
        .resources
        .iter()
        .map(|resource| {
            Datum::Vector(vec![
                Datum::String(resource.kind.clone()),
                number(resource.amount),
            ])
        })
        .collect();
    let datum = Datum::Node {
        tag: Symbol::qualified(NAMESPACE, "receiver-packet-v1"),
        fields: vec![
            (Symbol::qualified(NAMESPACE, "id"), packet.id().to_datum()),
            (Symbol::qualified(NAMESPACE, "bytes"), number(derived.bytes)),
            (
                Symbol::qualified(NAMESPACE, "files"),
                number(u64::from(derived.files)),
            ),
            (
                Symbol::qualified(NAMESPACE, "tokens"),
                number(derived.tokens),
            ),
            (
                Symbol::qualified(NAMESPACE, "paths"),
                Datum::Vector(derived.paths.iter().cloned().map(Datum::String).collect()),
            ),
            (
                Symbol::qualified(NAMESPACE, "role"),
                Datum::String(TARGET_ROLE.to_owned()),
            ),
            (
                Symbol::qualified(NAMESPACE, "status"),
                Datum::String(DECLARED_STATUS.to_owned()),
            ),
            (
                Symbol::qualified(NAMESPACE, "resources"),
                Datum::Vector(resources),
            ),
            (
                Symbol::qualified(NAMESPACE, "receiver"),
                Datum::String(derived.receiver.clone()),
            ),
        ],
    };
    datum
        .canonical_bytes()
        .map_err(|_| WorkReceiveError::Schema {
            mirrors_agree: false,
        })
}

fn command(phase: &str, verb: &str) -> Result<OwnerCommand, WorkReceiveError> {
    OwnerCommand::new(
        ".".into(),
        vec![verb.to_owned(), phase.to_owned()],
        "model".into(),
    )
    .map_err(|_| WorkReceiveError::Schema {
        mirrors_agree: false,
    })
}

fn surface_key(value: &str) -> Result<SurfaceKey, WorkReceiveError> {
    SurfaceKey::new(value).map_err(|_| WorkReceiveError::Schema {
        mirrors_agree: false,
    })
}

fn number(value: u64) -> Datum {
    Datum::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "u64"),
        canonical: value.to_string(),
    })
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
