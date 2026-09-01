use crate::candidate::content_id_text;
use crate::{CandidateRevision, IdentityVerification, verify_observed_identity};
use sim_kernel::{ContentId, Result};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidatePresence {
    Present,
    Absent,
}

#[derive(Clone, Debug)]
pub struct CandidateRecord {
    pub revision: CandidateRevision,
    pub presence: CandidatePresence,
}

#[derive(Clone, Debug)]
pub struct IdentityObservation {
    pub candidate: ContentId,
    pub verification: IdentityVerification,
}

/// Append-preserving candidate census. Sync marks absence but never deletes evidence.
#[derive(Clone, Debug, Default)]
pub struct CandidateCensus {
    records: BTreeMap<String, CandidateRecord>,
}

impl CandidateCensus {
    pub fn successful_sync(&mut self, present: Vec<CandidateRevision>) -> Result<()> {
        for record in self.records.values_mut() {
            record.presence = CandidatePresence::Absent;
        }
        for revision in present {
            let key = content_id_text(revision.subject_revision()?.content_id());
            self.records.insert(
                key,
                CandidateRecord {
                    revision,
                    presence: CandidatePresence::Present,
                },
            );
        }
        Ok(())
    }
    pub fn records(&self) -> impl Iterator<Item = &CandidateRecord> {
        self.records.values()
    }
    pub fn observe(
        &self,
        planned: &CandidateRevision,
        returned: &CandidateRevision,
    ) -> Result<IdentityObservation> {
        Ok(IdentityObservation {
            candidate: planned.subject_revision()?.content_id().clone(),
            verification: verify_observed_identity(planned, returned)?,
        })
    }
}
