use sim_roadmap_core::RoadmapRevision;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceDisposition {
    Candidate,
    ExactReusable,
    Invalidated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionDiff {
    pub path: FieldPath,
    pub evidence: EvidenceDisposition,
}

macro_rules! changed_field {
    ($changes:ident, $id:ident, $old:ident, $new:ident, $field:ident, $disposition:ident) => {
        if $old.$field != $new.$field {
            $changes.push(change(
                format!("phases.{}.{field}", $id, field = stringify!($field)),
                EvidenceDisposition::$disposition,
            ));
        }
    };
}

pub fn diff_revisions(old: &RoadmapRevision, new: &RoadmapRevision) -> Vec<RevisionDiff> {
    let mut changes = Vec::new();
    for (id, old_phase) in &old.spec.phases {
        match new.spec.phases.get(id) {
            None => changes.push(change(
                format!("phases.{id}"),
                EvidenceDisposition::Invalidated,
            )),
            Some(new_phase) if old_phase != new_phase => {
                changed_field!(changes, id, old_phase, new_phase, parent, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, title, Candidate);
                changed_field!(changes, id, old_phase, new_phase, intent, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, body, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, dependencies, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, owners, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, resources, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, effects, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, capabilities, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, changes, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, acceptance, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, coverage, Candidate);
                changed_field!(changes, id, old_phase, new_phase, outputs, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, guide, Invalidated);
                changed_field!(changes, id, old_phase, new_phase, origin, Invalidated);
            }
            Some(_) => changes.push(change(
                format!("phases.{id}"),
                EvidenceDisposition::ExactReusable,
            )),
        }
    }
    for id in new
        .spec
        .phases
        .keys()
        .filter(|id| !old.spec.phases.contains_key(*id))
    {
        changes.push(change(
            format!("phases.{id}"),
            EvidenceDisposition::Candidate,
        ));
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

fn change(path: String, evidence: EvidenceDisposition) -> RevisionDiff {
    RevisionDiff {
        path: FieldPath(path),
        evidence,
    }
}
