// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Ownership checks for a sealed fixed point.
//!
//! Each generated artifact has one owner and one generator. Exact paths are
//! the only region grain. A generator reads authored bytes and writes only
//! the paths that owner declares.

use std::collections::{BTreeMap, BTreeSet};

use sim_artifact_facet::{ArtifactFacet, ArtifactId, OwnerId, RegionOwnership, RegionSelector};
use sim_kernel::ContentId;

use crate::Revision;

use super::{FixedStep, Refusal, blocked, refusal};

/// One pure generator. `generate` receives only the facts named by [`Self::reads`].
pub trait FactGenerator {
    /// Owner identity. It matches the generated facet owner.
    fn owner(&self) -> &OwnerId;
    /// Authored fact paths this generator may read.
    fn reads(&self) -> &BTreeSet<String>;
    /// Exact paths this generator writes.
    fn writes(&self) -> &BTreeSet<String>;
    /// Qualified implementation. Equal values are the same computation.
    fn implementation(&self) -> ContentId;
    /// Postimage bytes for `writes`, derived from `source` alone.
    fn generate(&self, source: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>>;
}

pub(super) struct Checked {
    pub generated: BTreeMap<String, OwnerId>,
    writes: BTreeMap<OwnerId, BTreeSet<String>>,
}

pub(super) fn check(
    revision: &Revision,
    facets: &[ArtifactFacet],
    generators: &[&dyn FactGenerator],
) -> Result<Checked, Refusal> {
    if let Some(step) = line_grain(facets) {
        return Err(step);
    }
    let declared = declare(facets)?;
    let generated = declared.generated;
    let authored = declared.authored;
    let groups = declared.groups;
    if let Some(step) = duplicate_writes(generators) {
        return Err(step);
    }
    let writes = generator_writes(generators)?;
    if let Some(step) = cycles(generators, &generated) {
        return Err(step);
    }
    if let Some(step) = write_targets(revision, generators, &generated, &authored) {
        return Err(step);
    }
    bind_groups(&groups, &writes)?;
    Ok(Checked { generated, writes })
}

pub(super) fn invalidate(
    revision: &Revision,
    checked: &Checked,
    deleted: &BTreeSet<String>,
) -> FixedStep {
    let mut unknown = BTreeSet::new();
    let mut authored = BTreeSet::new();
    for path in deleted {
        if !revision.images().contains_key(path) {
            unknown.insert(path.clone());
        } else if !checked.generated.contains_key(path) {
            authored.insert(path.clone());
        }
    }
    if !unknown.is_empty() {
        return blocked("unknown-fact", unknown);
    }
    if !authored.is_empty() {
        return blocked("not-generated", authored);
    }
    let mut owners = BTreeSet::new();
    let mut write_paths = BTreeSet::new();
    for path in deleted {
        let Some(owner) = checked.generated.get(path) else {
            return blocked("split-generator", BTreeSet::from([path.clone()]));
        };
        let Some(writes) = checked.writes.get(owner) else {
            return blocked("split-generator", BTreeSet::from([path.clone()]));
        };
        owners.insert(owner.clone());
        write_paths.extend(writes.iter().cloned());
    }
    let mut closure = write_paths.clone();
    for (id, claim) in revision.claims() {
        if claim.inputs.iter().any(|input| write_paths.contains(input)) {
            closure.insert(id.clone());
        }
    }
    let mut retained = BTreeSet::new();
    for (owner, writes) in &checked.writes {
        if !owners.contains(owner) {
            retained.extend(writes.iter().cloned());
        }
    }
    FixedStep::Invalidated { closure, retained }
}

fn line_grain(facets: &[ArtifactFacet]) -> Option<Refusal> {
    let paths = facets
        .iter()
        .filter_map(|facet| match facet.region() {
            RegionSelector::LineRange { path, .. } => Some(path.clone()),
            RegionSelector::ExactPath(_) => None,
        })
        .collect::<BTreeSet<_>>();
    if paths.is_empty() {
        None
    } else {
        Some(refusal("region-grain", paths))
    }
}

#[derive(Default)]
struct Group {
    owners: BTreeSet<OwnerId>,
    paths: BTreeSet<String>,
}

struct Declared {
    generated: BTreeMap<String, OwnerId>,
    authored: BTreeSet<String>,
    groups: BTreeMap<ArtifactId, Group>,
}

fn declare(facets: &[ArtifactFacet]) -> Result<Declared, Refusal> {
    let mut generated = BTreeMap::new();
    let mut authored = BTreeSet::new();
    let mut groups: BTreeMap<ArtifactId, Group> = BTreeMap::new();
    for facet in facets {
        let RegionSelector::ExactPath(path) = facet.region() else {
            continue;
        };
        match facet.ownership() {
            RegionOwnership::Authored => {
                if generated.contains_key(path) || !authored.insert(path.clone()) {
                    return Err(refusal("split-generator", BTreeSet::from([path.clone()])));
                }
            }
            RegionOwnership::Generated { .. } => {
                if authored.contains(path) {
                    return Err(refusal("split-generator", BTreeSet::from([path.clone()])));
                }
                if generated
                    .insert(path.clone(), facet.owner().clone())
                    .is_some()
                {
                    return Err(refusal(
                        "duplicate-generator",
                        BTreeSet::from([path.clone()]),
                    ));
                }
                let group = groups.entry(facet.artifact().clone()).or_default();
                group.owners.insert(facet.owner().clone());
                group.paths.insert(path.clone());
            }
        }
    }
    for group in groups.values() {
        if group.owners.len() != 1 {
            return Err(refusal("split-generator", group.paths.clone()));
        }
    }
    Ok(Declared {
        generated,
        authored,
        groups,
    })
}

fn generator_writes(
    generators: &[&dyn FactGenerator],
) -> Result<BTreeMap<OwnerId, BTreeSet<String>>, Refusal> {
    let mut writes = BTreeMap::new();
    for generator in generators {
        if generator.writes().is_empty() {
            return Err(refusal("split-generator", BTreeSet::new()));
        }
        if let Some(existing) = writes.get(generator.owner()).cloned() {
            let mut closure: BTreeSet<String> = existing;
            closure.extend(generator.writes().iter().cloned());
            return Err(refusal("split-generator", closure));
        }
        writes.insert(generator.owner().clone(), generator.writes().clone());
    }
    Ok(writes)
}

fn duplicate_writes(generators: &[&dyn FactGenerator]) -> Option<Refusal> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for generator in generators {
        for path in generator.writes() {
            if !seen.insert(path.as_str()) {
                duplicates.insert(path.clone());
            }
        }
    }
    if duplicates.is_empty() {
        None
    } else {
        Some(refusal("duplicate-generator", duplicates))
    }
}

fn cycles(
    generators: &[&dyn FactGenerator],
    generated: &BTreeMap<String, OwnerId>,
) -> Option<Refusal> {
    let mut reads = BTreeSet::new();
    for generator in generators {
        for path in generator.reads() {
            if generated.contains_key(path) {
                reads.insert(path.clone());
            }
        }
    }
    if reads.is_empty() {
        None
    } else {
        Some(refusal("generator-cycle", reads))
    }
}

pub(super) fn absent_writes(
    revision: &Revision,
    generators: &[&dyn FactGenerator],
) -> BTreeSet<String> {
    generators
        .iter()
        .flat_map(|generator| generator.writes().iter())
        .filter(|path| !revision.images().contains_key((*path).as_str()))
        .cloned()
        .collect()
}

fn write_targets(
    revision: &Revision,
    generators: &[&dyn FactGenerator],
    generated: &BTreeMap<String, OwnerId>,
    authored: &BTreeSet<String>,
) -> Option<Refusal> {
    let mut authored_writes = BTreeSet::new();
    let mut splits = BTreeSet::new();
    for generator in generators {
        for path in generator.writes() {
            if authored.contains(path) {
                authored_writes.insert(path.clone());
            } else if let Some(owner) = generated.get(path)
                && owner != generator.owner()
            {
                splits.insert(path.clone());
            }
        }
    }
    let mut unknown = absent_writes(revision, generators);
    unknown.retain(|path| !authored.contains(path) && !splits.contains(path));
    if !authored_writes.is_empty() {
        return Some(refusal("authored-write", authored_writes));
    }
    if !splits.is_empty() {
        return Some(refusal("split-generator", splits));
    }
    if !unknown.is_empty() {
        return Some(refusal("unknown-fact", unknown));
    }
    None
}

fn bind_groups(
    groups: &BTreeMap<ArtifactId, Group>,
    writes: &BTreeMap<OwnerId, BTreeSet<String>>,
) -> Result<(), Refusal> {
    let mut used = BTreeSet::new();
    for group in groups.values() {
        let owner = group
            .owners
            .iter()
            .next()
            .ok_or_else(|| refusal("split-generator", group.paths.clone()))?;
        match writes.get(owner) {
            Some(paths) if paths == &group.paths => {
                used.insert(owner.clone());
            }
            Some(paths) => {
                return Err(refusal("split-generator", symdiff(paths, &group.paths)));
            }
            None => return Err(refusal("split-generator", group.paths.clone())),
        }
    }
    for (owner, paths) in writes {
        if !used.contains(owner) {
            return Err(refusal("split-generator", paths.clone()));
        }
    }
    Ok(())
}

fn symdiff(left: &BTreeSet<String>, right: &BTreeSet<String>) -> BTreeSet<String> {
    left.symmetric_difference(right).cloned().collect()
}
