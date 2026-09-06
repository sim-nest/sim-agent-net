use sim_conformance_core::{
    CheckInvocation, CheckerBinding, CheckerReceipt, CheckerReceiptId, RevocationStatus,
};

use super::{ProofCatalog, ProofError};

impl ProofCatalog {
    /// Adds one exact checker receipt after verifying its binding, invocation, and revocation.
    pub fn with_checker_receipt(
        mut self,
        retained: RetainedCheckerReceipt,
        revocation: RevocationStatus,
    ) -> Result<Self, ProofError> {
        retained.verify(revocation)?;
        let id = retained.receipt.id().clone();
        if self.checker_receipts.insert(id, retained).is_some() {
            return Err(ProofError::Invalid("duplicate checker receipt".into()));
        }
        Ok(self)
    }

    /// Retrieves and revalidates an exact checker receipt against current revocation state.
    pub fn checker_receipt(
        &self,
        id: &CheckerReceiptId,
        revocation: RevocationStatus,
    ) -> Result<&CheckerReceipt, ProofError> {
        let retained = self
            .checker_receipts
            .get(id)
            .ok_or(ProofError::CheckerReceiptNotCatalogued)?;
        retained.verify(revocation)?;
        Ok(&retained.receipt)
    }
}

/// Public records needed to retain and later revalidate one checker receipt.
#[derive(Clone, Debug)]
pub struct RetainedCheckerReceipt {
    binding: CheckerBinding,
    invocation: CheckInvocation,
    receipt: CheckerReceipt,
}

impl RetainedCheckerReceipt {
    /// Groups an already constructed binding, invocation, and receipt without weakening them.
    pub fn new(
        binding: CheckerBinding,
        invocation: CheckInvocation,
        receipt: CheckerReceipt,
    ) -> Self {
        Self {
            binding,
            invocation,
            receipt,
        }
    }

    /// Returns the retained receipt identity.
    pub const fn id(&self) -> &CheckerReceiptId {
        self.receipt.id()
    }

    fn verify(&self, revocation: RevocationStatus) -> Result<(), ProofError> {
        self.receipt
            .verify(&self.binding, &self.invocation, revocation)
            .map_err(|error| ProofError::InvalidCheckerReceipt(error.to_string()))
    }
}
