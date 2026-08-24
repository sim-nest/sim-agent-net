use std::collections::{BTreeMap, BTreeSet};

use crate::{CausalPath, Failure, PhaseBody, PhaseId, RoadmapSpec};

pub(crate) struct Tree {
    pub paths: BTreeMap<PhaseId, CausalPath>,
    pub descendants: BTreeMap<PhaseId, BTreeSet<PhaseId>>,
}

impl Tree {
    pub fn validate(spec: &RoadmapSpec) -> Result<Self, Failure> {
        if spec.phases.len() > spec.limits.phases {
            return Err(tree_failure(
                spec,
                "phase-limit",
                &spec.root,
                None,
                vec![spec.root.clone()],
            )?);
        }
        let root = spec
            .phases
            .get(&spec.root)
            .ok_or_else(|| Failure::Missing {
                kind: "root phase",
                id: spec.root.to_string(),
            })?;
        if root.parent.is_some() {
            return Err(tree_failure(
                spec,
                "root-has-parent",
                &spec.root,
                root.parent.clone(),
                vec![spec.root.clone()],
            )?);
        }
        let roots: Vec<_> = spec
            .phases
            .values()
            .filter(|p| p.parent.is_none())
            .map(|p| p.id.clone())
            .collect();
        if roots.len() != 1 || roots[0] != spec.root {
            let phase = roots.first().cloned().unwrap_or_else(|| spec.root.clone());
            return Err(tree_failure(
                spec,
                "exactly-one-root",
                &phase,
                None,
                vec![phase.clone()],
            )?);
        }

        let mut declared_parent = BTreeMap::new();
        let mut declared_children = BTreeSet::new();
        for phase in spec.phases.values() {
            if let Some(parent) = &phase.parent {
                if !spec.phases.contains_key(parent) {
                    return Err(tree_failure(
                        spec,
                        "missing-parent",
                        &phase.id,
                        Some(parent.clone()),
                        vec![phase.id.clone()],
                    )?);
                }
                declared_parent.insert(phase.id.clone(), parent.clone());
            }
            if let PhaseBody::Composite { children } = &phase.body {
                if children.len() > spec.limits.children_per_phase {
                    return Err(tree_failure(
                        spec,
                        "children-limit",
                        &phase.id,
                        None,
                        vec![phase.id.clone()],
                    )?);
                }
                if children.is_empty() {
                    return Err(tree_failure(
                        spec,
                        "empty-composite",
                        &phase.id,
                        None,
                        vec![phase.id.clone()],
                    )?);
                }
                for child in children {
                    if !declared_children.insert(child.clone()) {
                        return Err(Failure::Duplicate {
                            kind: "child",
                            id: child.to_string(),
                        });
                    }
                    if !spec.phases.contains_key(child) {
                        return Err(tree_failure(
                            spec,
                            "missing-child",
                            &phase.id,
                            Some(child.clone()),
                            vec![phase.id.clone()],
                        )?);
                    }
                    if declared_parent
                        .insert(child.clone(), phase.id.clone())
                        .is_some_and(|p| p != phase.id)
                    {
                        return Err(tree_failure(
                            spec,
                            "duplicate-child",
                            child,
                            Some(phase.id.clone()),
                            vec![phase.id.clone(), child.clone()],
                        )?);
                    }
                    if spec.phases[child].parent.as_ref() != Some(&phase.id) {
                        return Err(tree_failure(
                            spec,
                            "parent-child-mismatch",
                            child,
                            Some(phase.id.clone()),
                            vec![phase.id.clone(), child.clone()],
                        )?);
                    }
                }
            }
        }

        let mut paths = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        walk(spec, &spec.root, vec![], &mut visiting, &mut paths)?;
        if paths.len() != spec.phases.len() {
            let phase = spec
                .phases
                .keys()
                .find(|id| !paths.contains_key(*id))
                .expect("count differs")
                .clone();
            return Err(tree_failure(
                spec,
                "disconnected-phase",
                &phase,
                phase.parent(spec),
                vec![phase.clone()],
            )?);
        }
        let mut descendants: BTreeMap<_, BTreeSet<_>> = spec
            .phases
            .keys()
            .cloned()
            .map(|p| (p, BTreeSet::new()))
            .collect();
        for (phase, path) in &paths {
            for ancestor in path
                .phases()
                .iter()
                .take(path.phases().len().saturating_sub(1))
            {
                descendants
                    .get_mut(ancestor)
                    .expect("known phase")
                    .insert(phase.clone());
            }
        }
        Ok(Self { paths, descendants })
    }
}

fn walk(
    spec: &RoadmapSpec,
    id: &PhaseId,
    mut prefix: Vec<PhaseId>,
    visiting: &mut BTreeSet<PhaseId>,
    paths: &mut BTreeMap<PhaseId, CausalPath>,
) -> Result<(), Failure> {
    if !visiting.insert(id.clone()) {
        prefix.push(id.clone());
        return Err(tree_failure(spec, "cycle", id, None, prefix)?);
    }
    prefix.push(id.clone());
    if prefix.len() > spec.limits.tree_depth {
        return Err(tree_failure(spec, "tree-depth-limit", id, None, prefix)?);
    }
    paths.insert(id.clone(), CausalPath::new(prefix.clone(), spec.limits)?);
    if let PhaseBody::Composite { children } = &spec.phases[id].body {
        for child in children {
            walk(spec, child, prefix.clone(), visiting, paths)?;
        }
    }
    visiting.remove(id);
    Ok(())
}

fn tree_failure(
    spec: &RoadmapSpec,
    rule: &'static str,
    phase: &PhaseId,
    related: Option<PhaseId>,
    path: Vec<PhaseId>,
) -> Result<Failure, Failure> {
    Ok(Failure::Tree {
        rule,
        path: CausalPath::new(path, spec.limits)?,
        phase: phase.clone(),
        related,
    })
}

trait ParentLookup {
    fn parent(&self, spec: &RoadmapSpec) -> Option<PhaseId>;
}
impl ParentLookup for PhaseId {
    fn parent(&self, spec: &RoadmapSpec) -> Option<PhaseId> {
        spec.phases.get(self).and_then(|p| p.parent.clone())
    }
}
