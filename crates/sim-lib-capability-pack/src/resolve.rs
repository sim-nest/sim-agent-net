use std::collections::{BTreeMap, BTreeSet};

use crate::{CapabilityPack, ContentId};

/// Injected immutable Table/Dir-style pack lookup.
pub trait PackDir {
    /// Gets an object and its verified content id. Implementations must not rebuild it.
    fn get(&self, id: &ContentId) -> Option<(ContentId, CapabilityPack)>;
}

/// Deterministic import closure in dependency-first order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPack {
    /// Root identity.
    pub root: ContentId,
    /// Dependency-first unique packs.
    pub packs: Vec<(ContentId, CapabilityPack)>,
    /// Effective capability ceiling for each pack.
    pub ceilings: BTreeMap<ContentId, BTreeSet<sim_kernel::Symbol>>,
}

/// Import-graph refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// Missing object.
    Missing(ContentId),
    /// Directory returned unlike content.
    ContentMismatch {
        /// Requested id.
        requested: ContentId,
        /// Actual id.
        actual: ContentId,
    },
    /// Cycle path.
    Cycle(Vec<ContentId>),
    /// One alias names unlike content.
    AliasConflict(sim_kernel::Symbol),
    /// Malformed pack record.
    Malformed(String),
    /// Unsupported version.
    UnsupportedVersion(u64),
}

/// Resolves and topologically orders a pack closure, intersecting ceilings at every edge.
pub fn resolve(
    dir: &dyn PackDir,
    root: ContentId,
    ceiling: BTreeSet<sim_kernel::Symbol>,
) -> Result<ResolvedPack, ResolveError> {
    let mut out = ResolvedPack {
        root: root.clone(),
        packs: vec![],
        ceilings: BTreeMap::new(),
    };
    visit(
        dir,
        &root,
        &ceiling,
        &mut vec![],
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
        &mut out,
    )?;
    Ok(out)
}

fn visit(
    dir: &dyn PackDir,
    id: &ContentId,
    ceiling: &BTreeSet<sim_kernel::Symbol>,
    stack: &mut Vec<ContentId>,
    done: &mut BTreeSet<ContentId>,
    aliases: &mut BTreeMap<sim_kernel::Symbol, ContentId>,
    out: &mut ResolvedPack,
) -> Result<(), ResolveError> {
    if let Some(pos) = stack.iter().position(|v| v == id) {
        let mut cycle = stack[pos..].to_vec();
        cycle.push(id.clone());
        return Err(ResolveError::Cycle(cycle));
    }
    if done.contains(id) {
        let prior = out.ceilings.get_mut(id).expect("done pack has ceiling");
        *prior = prior.intersection(ceiling).cloned().collect();
        return Ok(());
    }
    let (actual, pack) = dir
        .get(id)
        .ok_or_else(|| ResolveError::Missing(id.clone()))?;
    if &actual != id {
        return Err(ResolveError::ContentMismatch {
            requested: id.clone(),
            actual,
        });
    }
    if ContentId::parse(pack.content.clone()).as_ref() != Ok(id) {
        return Err(ResolveError::ContentMismatch {
            requested: id.clone(),
            actual: ContentId::parse(pack.content.clone()).map_err(ResolveError::Malformed)?,
        });
    }
    if pack.version != crate::CURRENT_PACK_VERSION {
        return Err(ResolveError::UnsupportedVersion(pack.version));
    }
    stack.push(id.clone());
    for import in pack.typed_imports().map_err(ResolveError::Malformed)? {
        if let Some(old) = aliases.insert(import.alias.clone(), import.content.clone())
            && old != import.content
        {
            return Err(ResolveError::AliasConflict(import.alias));
        }
        let child = ceiling.intersection(&import.ceiling).cloned().collect();
        visit(dir, &import.content, &child, stack, done, aliases, out)?;
    }
    stack.pop();
    done.insert(id.clone());
    out.ceilings.insert(id.clone(), ceiling.clone());
    out.packs.push((id.clone(), pack));
    Ok(())
}
