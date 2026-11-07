// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sim_roadmap_core::{CheckpointId, RoadmapId};

use super::{AuthorError, FamilyMember, Stance};

/// Parse one self-contained source. The returned member keeps `bytes` exactly.
pub fn parse_member(bytes: &[u8]) -> Result<FamilyMember, AuthorError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AuthorError::Utf8)?;
    let mut header = false;
    let mut name = None;
    let mut round = None;
    let mut stance = None;
    let mut path = None;
    let mut argument = None;
    let mut work = None;
    let mut gate = None;
    let mut ratchet = None;
    let mut owner = None;
    let mut checker = None;
    let mut roadmap = None;
    let mut facts = Vec::new();
    let mut advice = Vec::new();
    let mut decisions = Vec::new();
    let mut consults = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        if !header {
            if line != "family/v1" {
                return Err(AuthorError::Header);
            }
            header = true;
            continue;
        }
        let (key, value) = match line.split_once(' ') {
            Some((key, value)) => (key, value.trim()),
            None => (line, ""),
        };
        match key {
            "name" => set(&mut name, "name", value)?,
            "round" => set(&mut round, "round", value)?,
            "stance" => set(&mut stance, "stance", value)?,
            "path" => set(&mut path, "path", value)?,
            "argument" => set(&mut argument, "argument", value)?,
            "work" => set(&mut work, "work", value)?,
            "gate" => set(&mut gate, "gate", value)?,
            "ratchet" => set(&mut ratchet, "ratchet", value)?,
            "owner" => set(&mut owner, "owner", value)?,
            "checker" => set(&mut checker, "checker", value)?,
            "roadmap" => set(&mut roadmap, "roadmap", value)?,
            "fact" => push_unique(&mut facts, "fact", value)?,
            "advice" => push(&mut advice, "advice", value)?,
            "decision" => push(&mut decisions, "decision", value)?,
            "consult" => push_unique(&mut consults, "consult", value)?,
            other => {
                return Err(AuthorError::UnknownField {
                    name: other.to_owned(),
                });
            }
        }
    }
    if !header {
        return Err(AuthorError::Header);
    }
    let name = required(name, "name")?;
    let round = required(round, "round")?;
    let stance = required(stance, "stance")?;
    let path = required(path, "path")?;
    let argument = required(argument, "argument")?;
    let work = required(work, "work")?;
    let gate = required(gate, "gate")?;
    let ratchet = required(ratchet, "ratchet")?;
    let owner = required(owner, "owner")?;
    let checker = required(checker, "checker")?;
    let roadmap = required(roadmap, "roadmap")?;
    token("name", &name)?;
    token("round", &round)?;
    for consult in &consults {
        token("consult", consult)?;
    }
    let stance = parse_stance(&stance)?;
    check_path(&path)?;
    RoadmapId::new(&roadmap).map_err(|_| AuthorError::Identity { kind: "roadmap" })?;
    check_surfaces(&owner, &roadmap)?;
    for fact in &facts {
        admit_fact(fact)?;
    }
    Ok(FamilyMember {
        source: bytes.to_vec(),
        name,
        round,
        stance,
        path,
        argument,
        work,
        gate,
        ratchet,
        owner,
        checker,
        roadmap,
        facts,
        advice,
        decisions,
        consults,
    })
}

fn set(slot: &mut Option<String>, section: &'static str, value: &str) -> Result<(), AuthorError> {
    if value.is_empty() {
        return Err(AuthorError::Missing { section });
    }
    if slot.is_some() {
        return Err(AuthorError::Duplicate { section });
    }
    *slot = Some(value.to_owned());
    Ok(())
}

fn push(out: &mut Vec<String>, section: &'static str, value: &str) -> Result<(), AuthorError> {
    if value.is_empty() {
        return Err(AuthorError::Missing { section });
    }
    out.push(value.to_owned());
    Ok(())
}

fn push_unique(
    out: &mut Vec<String>,
    section: &'static str,
    value: &str,
) -> Result<(), AuthorError> {
    if value.is_empty() {
        return Err(AuthorError::Missing { section });
    }
    if out.iter().any(|seen| seen == value) {
        return Err(AuthorError::Duplicate { section });
    }
    out.push(value.to_owned());
    Ok(())
}

fn required(value: Option<String>, section: &'static str) -> Result<String, AuthorError> {
    value.ok_or(AuthorError::Missing { section })
}

fn token(kind: &'static str, value: &str) -> Result<(), AuthorError> {
    let ok = !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if ok {
        Ok(())
    } else {
        Err(AuthorError::Identity { kind })
    }
}

fn parse_stance(value: &str) -> Result<Stance, AuthorError> {
    match value {
        "candidate" => Ok(Stance::Candidate),
        "consulted" => Ok(Stance::Consulted),
        "superseded" => Ok(Stance::Superseded),
        "selected" => Ok(Stance::Selected),
        _ => Err(AuthorError::Stance),
    }
}

/// Revision images share one path ceiling with mutation plans.
const MAX_PATH_BYTES: usize = 4096;

pub(super) fn check_path(path: &str) -> Result<(), AuthorError> {
    let seal = crate::SOURCE_SEAL;
    let reserved = path == seal
        || path
            .strip_prefix(seal)
            .is_some_and(|rest| rest.starts_with('/'));
    let refused = path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || reserved
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if refused {
        Err(AuthorError::Path)
    } else {
        Ok(())
    }
}

pub(super) fn check_surfaces(owner: &str, roadmap: &str) -> Result<(), AuthorError> {
    if !crate::packet_work::valid_name(owner) || owner == roadmap {
        return Err(AuthorError::Identity { kind: "owner" });
    }
    if !crate::packet_work::valid_name(roadmap) {
        return Err(AuthorError::Identity { kind: "roadmap" });
    }
    Ok(())
}

pub(super) fn admit_fact(fact: &str) -> Result<(), AuthorError> {
    CheckpointId::new(fact).map_err(|_| AuthorError::Identity { kind: "fact" })?;
    if crate::schedule::check_id(fact).is_err() {
        return Err(AuthorError::Identity { kind: "fact" });
    }
    Ok(())
}
