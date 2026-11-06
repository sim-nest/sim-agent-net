// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use thiserror::Error;

use crate::ExecutionJournalError;

/// Failure from a grounded proof leaf or from evidence admission.
#[derive(Debug, Error)]
pub enum ProofError {
    /// The named leaf is not in this catalog.
    #[error("proof leaf is not in the grounded catalog: {0}")]
    NotCatalogued(String),
    /// The leaf or sealed text is not a valid catalog input.
    #[error("invalid proof leaf: {0}")]
    Invalid(String),
    /// The checker receipt was never admitted to this catalog.
    #[error("checker receipt is absent from the grounded proof catalog")]
    CheckerReceiptNotCatalogued,
    /// The retained checker receipt does not match its binding.
    #[error("invalid checker receipt: {0}")]
    InvalidCheckerReceipt(String),
    /// Evidence admission or reuse refused for a named reason.
    #[error("evidence refused: {0}")]
    Refused(&'static str),
    /// The execution journal could not retain the proof attempt.
    #[error(transparent)]
    Journal(#[from] ExecutionJournalError),
    /// A launched effect has no conclusive launcher receipt.
    #[error("an already-launched proof effect has no conclusive launcher receipt")]
    AmbiguousEffect,
}
