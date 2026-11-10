// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Proof identity and the checker invocation that an evidence claim must match.

use sha2::{Digest, Sha256};
use sim_conformance_core::{CheckInputClosureId, CheckInvocation};
use sim_kernel::ContentId;
use sim_lib_exec::SandboxLimits;

use super::{DiscoveredDefinition, EvidenceClaim, ProofError};
use crate::{CommandProof, ProofLeaf};

pub(crate) fn require_invocation(
    invocation: &CheckInvocation,
    claim: &EvidenceClaim,
    definition: &str,
    code: &str,
    selected: &[&DiscoveredDefinition],
) -> Result<(), ProofError> {
    if invocation.subject() != &claim.subject || invocation.scope() != &claim.scope {
        return Err(ProofError::Refused("unrelated-invocation"));
    }
    let bound = attachment_closure(claim, definition, code, selected, true)?;
    if invocation.input_closure() == &bound {
        return Ok(());
    }
    let labels = attachment_closure(claim, definition, code, selected, false)?;
    if invocation.input_closure() == &labels {
        return Err(ProofError::Refused("changed-source-semantic"));
    }
    Err(ProofError::Refused("unrelated-invocation"))
}

pub(crate) fn attachment_closure(
    claim: &EvidenceClaim,
    definition: &str,
    code: &str,
    selected: &[&DiscoveredDefinition],
    bind_storage: bool,
) -> Result<CheckInputClosureId, ProofError> {
    let projections = digest(&sorted_join(&claim.projections));
    let environment = digest(claim.environment.as_str());
    let universe = digest(&universe_text(selected, bind_storage));
    let text = canon(&[
        ("subject", &hex(claim.subject.content_id())),
        ("scope", &hex(claim.scope.content_id())),
        ("proof", &digest(definition)),
        ("proof-code", &digest(code)),
        ("projections", &projections),
        ("environment", &environment),
        ("universe", &universe),
    ]);
    CheckInputClosureId::from_text(&text).map_err(|error| ProofError::Invalid(error.to_string()))
}

pub(crate) fn checker_preimage(binding: &str, code: &ContentId) -> String {
    let rendered = hex(code);
    canon(&[("binding", binding), ("code", &rendered)])
}

pub(crate) fn universe_text(selected: &[&DiscoveredDefinition], bind_storage: bool) -> String {
    let rows = selected
        .iter()
        .map(|row| {
            let storage = if bind_storage {
                row.storage.as_str()
            } else {
                ""
            };
            canon(&[("semantic", row.semantic.as_str()), ("storage", storage)])
        })
        .collect::<Vec<_>>();
    length_prefixed(&rows)
}

pub(crate) fn proof_definition(leaf: &ProofLeaf) -> String {
    let kind = match leaf {
        ProofLeaf::Command(_) => "command",
        ProofLeaf::ArtifactEquality { .. } => "artifact-equality",
        ProofLeaf::SourceDeckPredicate { .. } => "source-deck",
    };
    canon(&[("kind", kind), ("name", leaf.name())])
}

pub(crate) fn proof_code(leaf: &ProofLeaf) -> String {
    match leaf {
        ProofLeaf::Command(command) => command_code(command),
        ProofLeaf::ArtifactEquality { name, left, right } => {
            let left = hex_bytes(left);
            let right = hex_bytes(right);
            canon(&[("name", name.as_str()), ("left", &left), ("right", &right)])
        }
        ProofLeaf::SourceDeckPredicate {
            name,
            actual_deck,
            expected_deck,
            required_claims,
            present_claims,
        } => {
            let required = length_prefixed(&required_claims.iter().cloned().collect::<Vec<_>>());
            let present = length_prefixed(&present_claims.iter().cloned().collect::<Vec<_>>());
            canon(&[
                ("name", name.as_str()),
                ("actual", actual_deck.as_str()),
                ("expected", expected_deck.as_str()),
                ("required", &required),
                ("present", &present),
            ])
        }
    }
}

fn command_code(command: &CommandProof) -> String {
    let argv = length_prefixed(&command.argv);
    let env = length_prefixed(
        &command
            .environment
            .iter()
            .map(|(key, value)| canon(&[("key", key.as_str()), ("value", value.as_str())]))
            .collect::<Vec<_>>(),
    );
    let allowed = length_prefixed(
        &command
            .allowed_environment_keys
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
    );
    let limits = limit_text(&command.limits);
    let stdout = hex_bytes(&command.expected.stdout.content_id().bytes);
    let exit = command.expected.exit_code.to_string();
    let scratch = match &command.scratch_mount {
        Some(mount) => canon(&[("some", mount.as_str())]),
        None => canon(&[("none", "")]),
    };
    let read_only = if command.source_read_only {
        "true"
    } else {
        "false"
    };
    canon(&[
        ("name", command.name.as_str()),
        ("effect", command.effect_id.as_str()),
        ("program", command.program.as_str()),
        ("argv", &argv),
        ("cwd", command.working_directory.as_str()),
        ("env", &env),
        ("allowed", &allowed),
        ("mount", command.source_mount.as_str()),
        ("scratch", scratch.as_str()),
        ("read-only", read_only),
        ("limits", &limits),
        ("stdout", &stdout),
        ("exit", &exit),
    ])
}

fn limit_text(limits: &SandboxLimits) -> String {
    format!(
        "cpu={};memory={};wall={};processes={};files={};file-bytes={};output={};stdin={}",
        limits.cpu_seconds,
        limits.memory_bytes,
        limits.wall_time_ms,
        limits.process_count,
        limits.file_count,
        limits.file_bytes,
        limits.output_bytes,
        limits.stdin_bytes
    )
}

fn sorted_join(values: &[String]) -> String {
    let mut values = values.to_vec();
    values.sort();
    length_prefixed(&values)
}

fn length_prefixed(values: &[String]) -> String {
    let mut out = String::new();
    for value in values {
        out.push_str(&value.len().to_string());
        out.push(':');
        out.push_str(value);
        out.push(';');
    }
    out
}

fn canon(fields: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, value) in fields {
        out.push_str(name);
        out.push('=');
        out.push_str(&value.len().to_string());
        out.push(':');
        out.push_str(value);
        out.push(';');
    }
    out
}

fn hex(id: &ContentId) -> String {
    hex_bytes(&id.bytes)
}

fn digest(text: &str) -> String {
    hex_bytes(Sha256::digest(text.as_bytes()).as_slice())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
