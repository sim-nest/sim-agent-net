use sim_kernel::ContentId;

/// The party or boundary which can resolve a failure. Classification is data,
/// not prose interpreted by a retry loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureOwner {
    InputAuthor,
    Conduct,
    MutationAdapter,
    ProofSystem,
    Infrastructure,
    BudgetAuthority,
    AuthorityHolder,
    Unknown,
}

/// Closed recovery classes. New or unrecognised failures map to `Ambiguity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureClass {
    DeterministicInput,
    Conduct,
    Mutation,
    Proof,
    InfrastructureTransient,
    InfrastructurePermanent,
    Budget,
    Authority,
    Ambiguity,
}

impl FailureClass {
    pub const fn owner(self) -> FailureOwner {
        match self {
            Self::DeterministicInput => FailureOwner::InputAuthor,
            Self::Conduct => FailureOwner::Conduct,
            Self::Mutation => FailureOwner::MutationAdapter,
            Self::Proof => FailureOwner::ProofSystem,
            Self::InfrastructureTransient | Self::InfrastructurePermanent => {
                FailureOwner::Infrastructure
            }
            Self::Budget => FailureOwner::BudgetAuthority,
            Self::Authority => FailureOwner::AuthorityHolder,
            Self::Ambiguity => FailureOwner::Unknown,
        }
    }

    /// Intrinsic safety is deliberately narrower than policy permission.
    pub const fn intrinsically_retry_safe(self) -> bool {
        matches!(
            self,
            Self::Conduct | Self::Proof | Self::InfrastructureTransient
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassifiedFailure {
    pub class: FailureClass,
    pub evidence: Vec<ContentId>,
}

impl ClassifiedFailure {
    pub fn unknown(evidence: Vec<ContentId>) -> Self {
        Self {
            class: FailureClass::Ambiguity,
            evidence,
        }
    }
}
