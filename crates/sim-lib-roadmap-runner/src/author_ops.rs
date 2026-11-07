// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sim_roadmap_core::RoadmapSpec;
use sim_source_deck::SourceDeck;

use super::{
    AuthorError, Family, FamilyMember, MemberDiff, MemberRow, MemberView, ensure_roadmap,
    member_in_deck, member_roadmap_matches,
};

/// List every member with its deck and roadmap grounding.
pub fn list_family(
    family: &Family,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<Vec<MemberRow>, AuthorError> {
    ensure_roadmap(spec)?;
    Ok(family
        .members()
        .iter()
        .map(|member| MemberRow {
            name: member.name.clone(),
            round: member.round.clone(),
            stance: member.stance,
            path: member.path.clone(),
            roadmap: member.roadmap.clone(),
            in_deck: member_in_deck(deck, member),
            roadmap_matches: member_roadmap_matches(spec, member),
        })
        .collect())
}

/// Show one member and whether the deck and the spec ground it.
pub fn show_member(
    family: &Family,
    name: &str,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<MemberView, AuthorError> {
    ensure_roadmap(spec)?;
    let member = find(family, name)?;
    Ok(MemberView {
        in_deck: member_in_deck(deck, member),
        roadmap_matches: member_roadmap_matches(spec, member),
        member: member.clone(),
    })
}

/// Diff two members by section and by exact bytes.
pub fn diff_members(
    family: &Family,
    left: &str,
    right: &str,
    deck: &SourceDeck,
    spec: &RoadmapSpec,
) -> Result<MemberDiff, AuthorError> {
    ensure_roadmap(spec)?;
    let left_member = find(family, left)?;
    let right_member = find(family, right)?;
    Ok(MemberDiff {
        left: left_member.name.clone(),
        right: right_member.name.clone(),
        bytes_differ: left_member.source != right_member.source,
        changed: changed_sections(left_member, right_member),
        left_in_deck: member_in_deck(deck, left_member),
        right_in_deck: member_in_deck(deck, right_member),
        left_roadmap_matches: member_roadmap_matches(spec, left_member),
        right_roadmap_matches: member_roadmap_matches(spec, right_member),
    })
}

fn find<'a>(family: &'a Family, name: &str) -> Result<&'a FamilyMember, AuthorError> {
    family
        .members()
        .iter()
        .find(|member| member.name == name)
        .ok_or_else(|| AuthorError::MemberMissing {
            name: name.to_owned(),
        })
}

fn changed_sections(left: &FamilyMember, right: &FamilyMember) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if left.name != right.name {
        changed.push("name");
    }
    if left.round != right.round {
        changed.push("round");
    }
    if left.stance != right.stance {
        changed.push("stance");
    }
    if left.path != right.path {
        changed.push("path");
    }
    if left.argument != right.argument {
        changed.push("argument");
    }
    if left.work != right.work {
        changed.push("work");
    }
    if left.gate != right.gate {
        changed.push("gate");
    }
    if left.ratchet != right.ratchet {
        changed.push("ratchet");
    }
    if left.owner != right.owner {
        changed.push("owner");
    }
    if left.checker != right.checker {
        changed.push("checker");
    }
    if left.roadmap != right.roadmap {
        changed.push("roadmap");
    }
    if left.facts != right.facts {
        changed.push("fact");
    }
    if left.advice != right.advice {
        changed.push("advice");
    }
    if left.decisions != right.decisions {
        changed.push("decision");
    }
    if left.consults != right.consults {
        changed.push("consult");
    }
    changed
}
