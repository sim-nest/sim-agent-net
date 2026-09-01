use crate::CandidateRevision;
use sim_kernel::ContentId;
use sim_study_core::SubjectRevision;
use std::fmt;

impl CandidateRevision {
    /// Projects canonical candidate data to the opaque study subject identity.
    pub fn subject_revision(&self) -> sim_kernel::Result<SubjectRevision> {
        Ok(SubjectRevision::new(self.to_datum().content_id()?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityVerification {
    Verified,
    Quarantined(IdentityMismatch),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityMismatch {
    pub planned: ContentId,
    pub observed: ContentId,
}

impl fmt::Display for IdentityMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "returned model identity does not match the planned candidate; response quarantined",
        )
    }
}

/// Verifies every returned identity before its response may enter a study.
pub fn verify_observed_identity(
    planned: &CandidateRevision,
    observed: &CandidateRevision,
) -> sim_kernel::Result<IdentityVerification> {
    let planned = planned.to_datum().content_id()?;
    let observed = observed.to_datum().content_id()?;
    Ok(if planned == observed {
        IdentityVerification::Verified
    } else {
        IdentityVerification::Quarantined(IdentityMismatch { planned, observed })
    })
}
