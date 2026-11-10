// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bounded work admission at the packet receiver.
//!
//! One canonical input supplies lengths, identities, paths, roles, status,
//! resources, and the receiver selector. The typed packet builder materializes
//! declared bytes only after those checks pass. A model proposal is data: it
//! cannot carry an effect or an approval, and this function does not schedule
//! a second controller.

use sim_conformance_packet::{IdKind, SemanticId, StorageId};
use sim_work_core::{ImplementationPacket, WorkError};
use thiserror::Error;

use crate::{ModelWorker, ResourceAsk, WorkerView};

pub use sim_conformance_packet::InputPort;

pub(super) const NAMESPACE: &str = "work";
pub(super) const SCHEMA: &str = "shape/proposal";
pub(super) const SOURCE_ROLE: &str = "released-dependency";
pub(super) const TARGET_ROLE: &str = "funded-target";
pub(super) const DECLARED_STATUS: &str = "declared";

#[path = "packet_work_build.rs"]
mod build;
#[path = "packet_work_check.rs"]
mod check;

pub(crate) use check::valid_name;
#[path = "packet_work_decode.rs"]
mod decode;

/// One declared source. Its path and body are the canonical bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkSource {
    /// Surface that owns this source.
    pub surface: String,
    /// Relative path carried into the receiver record.
    pub path: String,
    /// Exact source bytes. The length is `body.len()`.
    pub body: Vec<u8>,
}

/// Canonical work input. Derived fields are not supplied beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalWork {
    /// Packet namespace. Only `work` is admitted.
    pub namespace: String,
    /// Packet phase text.
    pub phase: String,
    /// Owner law name.
    pub law: String,
    /// Owning package name.
    pub owner_package: String,
    /// Declared sources, in identity order before sorting by surface.
    pub sources: Vec<WorkSource>,
    /// Role claimed for every source.
    pub source_role: String,
    /// Funded target surface.
    pub target_surface: String,
    /// Role claimed for the funded target.
    pub target_role: String,
    /// Status claimed for the funded target.
    pub target_status: String,
    /// Input byte ceiling.
    pub budget_bytes: u64,
    /// Input file ceiling.
    pub budget_files: u32,
    /// Token ceiling. Tokens are the derived byte lengths.
    pub budget_tokens: u64,
    /// Output byte ceiling.
    pub budget_output: u64,
    /// Resource offers derived into the receiver record.
    pub resources: Vec<ResourceAsk>,
    /// Copy generation presented with the input.
    pub copy: String,
    /// Current generation the receiver requires.
    pub current: String,
    /// Output schema text.
    pub schema: String,
    /// Mirror agreement. It is reported and never authorizes admission.
    pub mirrors_agree: bool,
    /// Attempt ceiling, including the first return.
    pub max_attempts: u32,
    /// Unresolved measure before this packet.
    pub unresolved: u64,
    /// Unresolved measure this packet claims after admission.
    pub narrowed_to: u64,
}

/// Modeled return body. Bytes are data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelBody {
    /// A proposal the receiver decodes.
    Proposal(Vec<u8>),
    /// Partial output kept with the packet evidence.
    Incomplete(Vec<u8>),
    /// Bytes that do not decode.
    Malformed(Vec<u8>),
}

/// One modeled return, including the serialization the receiver observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelReturn {
    /// Serialization the model claims the receiver sent.
    pub seen: Vec<u8>,
    /// Proposal, partial output, or malformed bytes.
    pub body: ModelBody,
    /// Unresolved measure after this return.
    pub unresolved: u64,
}

/// Bound model proposals. This is not a scheduler.
pub trait ProposalModel {
    /// Return the modeled proposal for one receiver and attempt.
    fn propose(&self, receiver: &str, attempt: u32, serialized: &[u8]) -> ModelReturn;
}

/// Packet evidence retained for an admitted or incomplete result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedWork {
    /// Canonical packet identity, hex-encoded.
    pub packet_id: String,
    /// Canonical receiver serialization.
    pub serialized: Vec<u8>,
    /// Derived input byte length.
    pub bytes: u64,
    /// Derived file count.
    pub files: u32,
    /// Derived token count.
    pub tokens: u64,
    /// Derived relative paths.
    pub paths: Vec<String>,
    /// Derived target role.
    pub role: String,
    /// Derived target status.
    pub status: String,
    /// Derived resource offers.
    pub resources: Vec<ResourceAsk>,
    /// Derived receiver selector.
    pub receiver: String,
}

/// Result of receiving one canonical work input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkReceipt {
    /// The proposal decoded and named no effect or approval.
    Admitted {
        /// Retained packet evidence.
        work: CapturedWork,
        /// Decoded proposal value.
        proposal: String,
        /// Attempts used, including the first.
        attempts: u32,
    },
    /// Output stopped early. The packet evidence stays.
    Incomplete {
        /// Retained packet evidence.
        work: CapturedWork,
        /// Partial bytes. Empty when the worker has not returned.
        partial: Vec<u8>,
        /// Attempts used, including the first.
        attempts: u32,
    },
}

/// Why the receiver refused the input or the modeled return.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WorkReceiveError {
    /// The namespace differed from `work`.
    #[error("namespace refused")]
    Namespace {
        /// Whether the mirrors agreed with the refused namespace.
        mirrors_agree: bool,
    },
    /// A source or target role was not the role that surface requires.
    #[error("role refused")]
    Role {
        /// Whether the mirrors agreed with the refused role.
        mirrors_agree: bool,
    },
    /// The presented copy was not the current generation.
    #[error("stale copy refused")]
    StaleCopy {
        /// Whether the mirrors agreed with the stale copy.
        mirrors_agree: bool,
    },
    /// The schema, path, or receiver serialization did not match.
    #[error("schema refused")]
    Schema {
        /// Whether the mirrors agreed with the refused schema.
        mirrors_agree: bool,
    },
    /// The target status was not the declared status.
    #[error("status refused")]
    Status {
        /// Whether the mirrors agreed with the refused status.
        mirrors_agree: bool,
    },
    /// Derived input exceeded a ceiling before the port was read.
    #[error("oversize input refused")]
    Oversize,
    /// A retry did not strictly narrow the unresolved measure.
    #[error("retry had no descent")]
    NoDescent {
        /// Malformed bytes retained with the refusal.
        evidence: Vec<u8>,
    },
    /// Malformed returns reached the attempt ceiling.
    #[error("malformed retries are exhausted")]
    MalformedBound {
        /// Attempts used.
        attempts: u32,
        /// Last malformed bytes.
        evidence: Vec<u8>,
    },
    /// A model body was longer than the output ceiling.
    #[error("model output exceeded the byte ceiling")]
    OutputBudget {
        /// Attempts used, including the first.
        attempts: u32,
        /// Prefix of the body, no longer than the output ceiling.
        capture: Vec<u8>,
        /// Full length of the returned body.
        length: u64,
        /// Identity of the full body.
        identity: StorageId,
    },
    /// The decoded proposal carried an effect or an approval.
    #[error("proposal holds an effect or approval")]
    EffectsOrApproval,
    /// The typed builder refused the packet.
    #[error("packet refused: {0}")]
    Packet(String),
    /// The deterministic worker reported failure. The serialization is kept.
    #[error("worker failed")]
    WorkerFailed {
        /// Receiver serialization retained with the failure.
        evidence: Vec<u8>,
    },
}

struct Derived {
    bytes: u64,
    files: u32,
    tokens: u64,
    paths: Vec<String>,
    resources: Vec<ResourceAsk>,
    receiver: String,
}

/// Admit one canonical work input through the typed packet builder.
///
/// Oversize input, a wrong namespace, role, copy, status, or schema, and a
/// packet that does not narrow all return before the input port is read.
pub fn receive_work(
    input: &CanonicalWork,
    port: &mut dyn InputPort,
    worker: &dyn ModelWorker,
    proposals: &dyn ProposalModel,
) -> Result<WorkReceipt, WorkReceiveError> {
    let derived = check::precheck(input)?;
    let packet = build::build_packet(input, port)?;
    let work = build::captured(&packet, &derived)?;
    admit_returns(input, &derived, &packet, &work, worker, proposals)
}

pub(super) fn text_id<K: IdKind>(
    value: &str,
) -> Result<SemanticId<K>, sim_conformance_packet::ConformanceError> {
    SemanticId::from_text(value)
}

pub(super) fn map_work(error: WorkError) -> WorkReceiveError {
    match error {
        WorkError::ByteBudget | WorkError::FileBudget | WorkError::TokenBudget => {
            WorkReceiveError::Oversize
        }
        WorkError::NoDescent => WorkReceiveError::NoDescent {
            evidence: Vec::new(),
        },
        other => WorkReceiveError::Packet(other.to_string()),
    }
}

fn admit_returns(
    input: &CanonicalWork,
    derived: &Derived,
    packet: &ImplementationPacket,
    work: &CapturedWork,
    worker: &dyn ModelWorker,
    proposals: &dyn ProposalModel,
) -> Result<WorkReceipt, WorkReceiveError> {
    let mut remaining = input.narrowed_to;
    let mut attempt = 0u32;
    loop {
        attempt = attempt
            .checked_add(1)
            .ok_or(WorkReceiveError::MalformedBound {
                attempts: u32::MAX,
                evidence: Vec::new(),
            })?;
        if attempt > input.max_attempts {
            return Err(WorkReceiveError::MalformedBound {
                attempts: input.max_attempts,
                evidence: Vec::new(),
            });
        }
        match worker.outcome(&derived.receiver) {
            WorkerView::Blocked => {
                return Ok(WorkReceipt::Incomplete {
                    work: work.clone(),
                    partial: Vec::new(),
                    attempts: attempt,
                });
            }
            WorkerView::Failed => {
                return Err(WorkReceiveError::WorkerFailed {
                    evidence: work.serialized.clone(),
                });
            }
            WorkerView::Passed => {}
        }
        let returned = proposals.propose(&derived.receiver, attempt, &work.serialized);
        if returned.seen != work.serialized {
            return Err(WorkReceiveError::Schema {
                mirrors_agree: input.mirrors_agree,
            });
        }
        let body_bytes = match &returned.body {
            ModelBody::Incomplete(body)
            | ModelBody::Proposal(body)
            | ModelBody::Malformed(body) => body.as_slice(),
        };
        if u64::try_from(body_bytes.len())
            .map(|len| len > input.budget_output)
            .unwrap_or(true)
        {
            let limit = usize::try_from(input.budget_output).unwrap_or(usize::MAX);
            return Err(WorkReceiveError::OutputBudget {
                attempts: attempt,
                capture: body_bytes.iter().take(limit).copied().collect(),
                length: u64::try_from(body_bytes.len()).unwrap_or(u64::MAX),
                identity: StorageId::for_bytes(body_bytes),
            });
        }
        match returned.body {
            ModelBody::Incomplete(partial) => {
                return Ok(WorkReceipt::Incomplete {
                    work: work.clone(),
                    partial,
                    attempts: attempt,
                });
            }
            ModelBody::Proposal(body) => match decode::decode_body(input, packet, &body)? {
                decode::Decoded::Value(proposal) => {
                    return Ok(WorkReceipt::Admitted {
                        work: work.clone(),
                        proposal,
                        attempts: attempt,
                    });
                }
                decode::Decoded::Malformed => {
                    decode::retry_or_stop(
                        input,
                        &mut remaining,
                        attempt,
                        &body,
                        returned.unresolved,
                    )?;
                }
            },
            ModelBody::Malformed(body) => {
                decode::retry_or_stop(input, &mut remaining, attempt, &body, returned.unresolved)?;
            }
        }
    }
}

#[cfg(test)]
#[path = "packet_work_more_tests.rs"]
mod packet_work_more_tests;
#[cfg(test)]
#[path = "packet_work_tests.rs"]
mod packet_work_tests;
#[cfg(test)]
#[path = "packet_work_test_support.rs"]
mod support;
