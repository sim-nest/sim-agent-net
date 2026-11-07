// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeSet;

use sim_kernel::ContentId;
use sim_roadmap_core::{PhaseBody, RoadmapSpec};
use sim_source_deck::SourceDeck;

use super::parse::{admit_fact, check_path, check_surfaces};
use super::{
    AuthorError, Family, FamilyMember, Selection, Stance, ensure_roadmap, member_in_deck,
    member_roadmap_matches,
};
use crate::seal_id;

pub(super) struct Prepared<'a> {
    pub selected: &'a FamilyMember,
    pub required: BTreeSet<String>,
    pub consultation: Vec<ContentId>,
}

/// Refuse unless the family, deck, and spec satisfy the authoring gate.
pub fn lint_family(
    family: &Family,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<(), AuthorError> {
    prepare(family, deck, spec).map(|_| ())
}

/// Return the one selected source. The value supplies neither begin authority
/// nor hold release.
pub fn select_member(
    family: &Family,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<Selection, AuthorError> {
    let prepared = prepare(family, deck, spec)?;
    Ok(Selection {
        name: prepared.selected.name.clone(),
        round: prepared.selected.round.clone(),
        source: prepared.selected.source.clone(),
    })
}

pub(super) fn prepare<'a>(
    family: &'a Family,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<Prepared<'a>, AuthorError> {
    ensure_roadmap(spec)?;
    let selected = one_selected(family)?;
    for member in family.members() {
        check_path(&member.path)?;
        check_surfaces(&member.owner, &member.roadmap)?;
        if !member_roadmap_matches(spec, member) {
            return Err(AuthorError::RoadmapMismatch {
                found: member.roadmap.clone(),
                spec: spec.id.as_str().to_owned(),
            });
        }
        if !member_in_deck(deck, member) {
            return Err(AuthorError::DeckMismatch {
                path: member.path.clone(),
            });
        }
    }
    let required = checkpoints(spec);
    if required.is_empty() {
        return Err(AuthorError::Ungrounded);
    }
    for requirement in &required {
        admit_fact(requirement)?;
    }
    for fact in &selected.facts {
        admit_fact(fact)?;
    }
    for requirement in &required {
        if !selected.facts.iter().any(|fact| fact == requirement) {
            return Err(AuthorError::Unsatisfied {
                requirement: requirement.clone(),
            });
        }
    }
    if selected.decisions.iter().any(|text| names_authority(text)) {
        return Err(AuthorError::Authority);
    }
    check_consults(family)?;
    Ok(Prepared {
        selected,
        required,
        consultation: consultation_ids(family)?,
    })
}

fn one_selected(family: &Family) -> Result<&FamilyMember, AuthorError> {
    let mut selected = None;
    let mut count = 0usize;
    for member in family.members() {
        if member.stance == Stance::Selected {
            count += 1;
            selected = Some(member);
        }
    }
    if count != 1 {
        return Err(AuthorError::SelectionCount { count });
    }
    selected.ok_or(AuthorError::SelectionCount { count })
}

fn checkpoints(spec: &RoadmapSpec) -> BTreeSet<String> {
    let mut required = BTreeSet::new();
    for phase in spec.phases.values() {
        if let PhaseBody::Leaf { checkpoints } = &phase.body {
            for checkpoint in checkpoints {
                required.insert(checkpoint.id.as_str().to_owned());
            }
        }
    }
    required
}

fn check_consults(family: &Family) -> Result<(), AuthorError> {
    for member in family.members() {
        for round in &member.consults {
            let consulted = family
                .members()
                .iter()
                .any(|other| other.round == *round && other.stance == Stance::Consulted);
            if !consulted {
                return Err(AuthorError::Consult {
                    round: round.clone(),
                });
            }
        }
    }
    Ok(())
}

fn consultation_ids(family: &Family) -> Result<Vec<ContentId>, AuthorError> {
    family
        .members()
        .iter()
        .filter(|member| member.stance == Stance::Consulted)
        .map(|member| {
            seal_id(&member.source).map_err(|error| AuthorError::Revision(error.to_string()))
        })
        .collect()
}

fn names_authority(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    tokens.iter().any(|token| {
        matches!(
            *token,
            "begin-authority" | "hold-release" | "begin_authority" | "hold_release"
        )
    }) || tokens.windows(2).any(|pair| {
        (pair[0] == "begin" && pair[1] == "authority")
            || (pair[0] == "hold" && pair[1] == "release")
    })
}
