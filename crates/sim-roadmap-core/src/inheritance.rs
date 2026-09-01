use std::collections::{BTreeMap, BTreeSet};

use crate::{CausalPath, Failure, PhaseId};

pub(crate) fn narrowed<T: Ord + ToString + Clone>(
    ancestor: &BTreeSet<T>,
    child: &BTreeSet<T>,
    ancestor_id: &PhaseId,
    child_id: &PhaseId,
    field: &'static str,
    path: &CausalPath,
) -> Result<BTreeSet<T>, Failure> {
    if let Some(value) = child.difference(ancestor).next() {
        return Err(Failure::Widening {
            ancestor: ancestor_id.clone(),
            phase: child_id.clone(),
            field,
            value: value.to_string(),
            path: path.clone(),
        });
    }
    Ok(child.clone())
}

pub(crate) fn narrowed_owners(
    ancestor_mutable: &BTreeSet<crate::OwnerId>,
    ancestor_read: &BTreeSet<crate::OwnerId>,
    child_mutable: &BTreeSet<crate::OwnerId>,
    child_read: &BTreeSet<crate::OwnerId>,
    ancestor_id: &PhaseId,
    child_id: &PhaseId,
    path: &CausalPath,
) -> Result<(), Failure> {
    narrowed(
        ancestor_mutable,
        child_mutable,
        ancestor_id,
        child_id,
        "owners.mutable",
        path,
    )?;
    let allowed: BTreeSet<_> = ancestor_mutable.union(ancestor_read).cloned().collect();
    narrowed(
        &allowed,
        child_read,
        ancestor_id,
        child_id,
        "owners.read_only",
        path,
    )?;
    Ok(())
}

pub(crate) fn acceptance_map<'a>(
    items: impl Iterator<Item = (&'a PhaseId, &'a crate::AcceptanceContract)>,
) -> BTreeMap<PhaseId, crate::AcceptanceContract> {
    items
        .map(|(id, acceptance)| (id.clone(), acceptance.clone()))
        .collect()
}
