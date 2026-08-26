use crate::candidate::content_id_text;
use crate::{CandidateCensus, CandidatePresence};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfflineSnapshot {
    text: String,
}
impl OfflineSnapshot {
    pub fn from_census(
        census: &CandidateCensus,
        forbidden: &[&str],
    ) -> Result<Self, SnapshotError> {
        let mut rows = Vec::new();
        for record in census.records() {
            let id = record
                .revision
                .subject_revision()
                .map_err(|_| SnapshotError)?;
            rows.push(format!(
                "{}\t{}\t{:?}\t{:?}",
                content_id_text(id.content_id()),
                record.revision.model,
                record.revision.route.semantics,
                record.presence
            ));
        }
        rows.sort();
        let text = rows.join("\n");
        if forbidden
            .iter()
            .any(|secret| !secret.is_empty() && text.contains(secret))
        {
            return Err(SnapshotError);
        }
        Ok(Self { text })
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotError;
impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("offline model snapshot failed privacy validation")
    }
}
impl std::error::Error for SnapshotError {}

#[allow(dead_code)]
fn _presence_is_public(_: CandidatePresence) {}
