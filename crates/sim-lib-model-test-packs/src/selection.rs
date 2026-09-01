use crate::PackError;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionStage {
    Smoke,
    Screen,
    Confirmation,
    FullReproduction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionRevision {
    pub name: String,
    pub revision: String,
    pub stage: SelectionStage,
    pub task_revisions: Vec<String>,
}
impl SelectionRevision {
    pub fn validate(&self) -> Result<(), PackError> {
        if self.name.is_empty()
            || self.revision.is_empty()
            || self.revision == "current"
            || self.task_revisions.is_empty()
            || self.task_revisions.iter().any(|x| x == "current")
        {
            return Err(PackError::Selection);
        }
        let unique: BTreeSet<_> = self.task_revisions.iter().collect();
        if unique.len() != self.task_revisions.len() {
            Err(PackError::Duplicate("selection task"))
        } else {
            Ok(())
        }
    }
}
