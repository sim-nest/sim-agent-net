// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Receiver decoding of modeled proposal bytes.

use sim_kernel::{Datum, Symbol};
use sim_work_core::{
    DescentCertificate, FacetPlanId, ImplementationPacket, WorkError, decode_proposal,
};

use super::{CanonicalWork, NAMESPACE, WorkReceiveError, text_id};

pub(super) enum Decoded {
    Value(String),
    Malformed,
}

pub(super) fn decode_body(
    input: &CanonicalWork,
    packet: &ImplementationPacket,
    body: &[u8],
) -> Result<Decoded, WorkReceiveError> {
    let facet: FacetPlanId = text_id("facet/plan").map_err(|_| WorkReceiveError::Schema {
        mirrors_agree: input.mirrors_agree,
    })?;
    match decode_proposal(
        packet,
        body,
        decode_model,
        shape_matches,
        facet,
        Vec::<sim_work_core::ReceiptId>::new(),
    ) {
        Ok(checked) => Ok(Decoded::Value(proposal_text(checked.decoded())?)),
        Err(WorkError::MalformedReturn(errors))
            if errors.iter().any(|item| item == "effects-or-approval") =>
        {
            Err(WorkReceiveError::EffectsOrApproval)
        }
        Err(WorkError::OutputBudget) => Err(WorkReceiveError::Schema {
            mirrors_agree: input.mirrors_agree,
        }),
        Err(WorkError::MalformedReturn(_)) => Ok(Decoded::Malformed),
        Err(other) => Err(super::map_work(other)),
    }
}

fn shape_matches(datum: &Datum, _contract: &sim_work_core::OutputContractId) -> Result<(), String> {
    if holds_authority(datum) {
        return Err("effects-or-approval".into());
    }
    proposal_text(datum)
        .map(|_| ())
        .map_err(|_| "schema".into())
}

fn proposal_text(datum: &Datum) -> Result<String, WorkReceiveError> {
    let Datum::Node { tag, fields } = datum else {
        return Err(WorkReceiveError::Schema {
            mirrors_agree: false,
        });
    };
    if tag != &Symbol::qualified(NAMESPACE, "proposal-v1") || fields.len() != 1 {
        return Err(WorkReceiveError::Schema {
            mirrors_agree: false,
        });
    }
    if fields[0].0 != Symbol::qualified(NAMESPACE, "value") {
        return Err(WorkReceiveError::Schema {
            mirrors_agree: false,
        });
    }
    match &fields[0].1 {
        Datum::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(WorkReceiveError::Schema {
            mirrors_agree: false,
        }),
    }
}

pub(super) fn retry_or_stop(
    input: &CanonicalWork,
    remaining: &mut u64,
    attempt: u32,
    body: &[u8],
    unresolved: u64,
) -> Result<(), WorkReceiveError> {
    if attempt >= input.max_attempts {
        return Err(WorkReceiveError::MalformedBound {
            attempts: attempt,
            evidence: body.to_vec(),
        });
    }
    DescentCertificate {
        measure: Symbol::qualified(NAMESPACE, "unknowns"),
        before: *remaining,
        after: unresolved,
    }
    .verify()
    .map_err(|_| WorkReceiveError::NoDescent {
        evidence: body.to_vec(),
    })?;
    *remaining = unresolved;
    Ok(())
}

fn decode_model(body: &[u8]) -> Result<Datum, String> {
    let text = std::str::from_utf8(body).map_err(|_| "schema".to_owned())?;
    let mut lines = text.lines();
    if lines.next() != Some("proposal/v1") {
        return Err("schema".into());
    }
    let mut fields = Vec::new();
    for line in lines {
        let Some((key, value)) = line.split_once(' ') else {
            return Err("schema".into());
        };
        if value.is_empty() || key.contains('/') {
            return Err("schema".into());
        }
        fields.push((
            Symbol::qualified(NAMESPACE, key),
            Datum::String(value.to_owned()),
        ));
    }
    Ok(Datum::Node {
        tag: Symbol::qualified(NAMESPACE, "proposal-v1"),
        fields,
    })
}

fn holds_authority(datum: &Datum) -> bool {
    match datum {
        Datum::Symbol(symbol) => authority_symbol(symbol),
        Datum::String(value) => value == "effect" || value == "approval",
        Datum::Node { tag, fields } => {
            authority_symbol(tag)
                || fields
                    .iter()
                    .any(|(name, value)| authority_symbol(name) || holds_authority(value))
        }
        Datum::Vector(items) | Datum::List(items) => items.iter().any(holds_authority),
        _ => false,
    }
}

fn authority_symbol(symbol: &Symbol) -> bool {
    symbol.name.as_ref() == "effect"
        || symbol.name.as_ref() == "approval"
        || symbol.namespace.as_deref() == Some("effect")
        || symbol.namespace.as_deref() == Some("approval")
}
