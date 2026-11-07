// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Self-contained authoring over a source deck and a roadmap spec.
//!
//! A family is a set of exact source bytes. List, show, lint, select, and
//! diff read those bytes against the deck and the spec. Compile opens one
//! revision, fills one work value, builds one source-deck proof leaf, and
//! seals one mutation plan. Consulted and superseded siblings stay provenance.

use sim_kernel::{Expr, Symbol};
use sim_roadmap_core::RoadmapSpec;
use sim_shape::ExactExprShape;
use sim_source_deck::SourceDeck;
use thiserror::Error;

#[path = "author_check.rs"]
mod check;
#[path = "author_compile.rs"]
mod compile;
#[path = "author_ops.rs"]
mod ops;
#[path = "author_parse.rs"]
mod parse;

pub use check::{lint_family, select_member};
pub use compile::compile_authoring;
pub use ops::{diff_members, list_family, show_member};
pub use parse::parse_member;

/// How a family member stands toward the selected source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    /// A sibling that is not selected and is not consultation.
    Candidate,
    /// Provenance. Its facts do not satisfy a requirement.
    Consulted,
    /// A sibling that has been replaced. Its facts do not satisfy a requirement.
    Superseded,
    /// The one source compile may turn into revision, work, proof, and mutation values.
    Selected,
}

impl Stance {
    /// Source token for this stance.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Consulted => "consulted",
            Self::Superseded => "superseded",
            Self::Selected => "selected",
        }
    }
}

/// Kind of authored statement. Each kind has its own shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatementKind {
    /// Guidance that does not satisfy a requirement.
    Advice,
    /// A requirement the selected source itself names.
    Fact,
    /// A selection statement. It supplies neither begin authority nor hold release.
    Decision,
}

/// Shape for one statement kind.
///
/// Advice, fact, and decision are three exact shapes. None of them is a
/// subshape of another.
pub fn statement_shape(kind: StatementKind) -> ExactExprShape {
    let name = match kind {
        StatementKind::Advice => "advice",
        StatementKind::Fact => "fact",
        StatementKind::Decision => "decision",
    };
    ExactExprShape::new(Expr::Symbol(Symbol::qualified("authoring", name)))
}

/// One self-contained family member. `source` is the exact input bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyMember {
    /// Exact source bytes, unchanged by parsing.
    pub source: Vec<u8>,
    /// Member name.
    pub name: String,
    /// Member round.
    pub round: String,
    /// Stance of this member.
    pub stance: Stance,
    /// Deck path whose bytes must equal `source`.
    pub path: String,
    /// Argument section.
    pub argument: String,
    /// Work section.
    pub work: String,
    /// Gate section.
    pub gate: String,
    /// Ratchet section.
    pub ratchet: String,
    /// Owner section.
    pub owner: String,
    /// Checker section.
    pub checker: String,
    /// Roadmap id text.
    pub roadmap: String,
    /// Fact ids this member itself names.
    pub facts: Vec<String>,
    /// Advice statements.
    pub advice: Vec<String>,
    /// Decision statements.
    pub decisions: Vec<String>,
    /// Consultation rounds named by this member.
    pub consults: Vec<String>,
}

/// A family of self-contained sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    members: Vec<FamilyMember>,
}

impl Family {
    /// Assemble members. Names and rounds are unique. The family is non-empty.
    pub fn from_members(members: Vec<FamilyMember>) -> Result<Self, AuthorError> {
        if members.is_empty() {
            return Err(AuthorError::EmptyFamily);
        }
        let mut names = std::collections::BTreeSet::new();
        let mut rounds = std::collections::BTreeSet::new();
        for member in &members {
            if !names.insert(member.name.clone()) {
                return Err(AuthorError::DuplicateMember { kind: "name" });
            }
            if !rounds.insert(member.round.clone()) {
                return Err(AuthorError::DuplicateMember { kind: "round" });
            }
        }
        Ok(Self { members })
    }

    /// Members in the order they were assembled.
    pub fn members(&self) -> &[FamilyMember] {
        &self.members
    }
}

/// Exact source bytes. Parsing does not rewrite them.
pub fn source_round_trip(member: &FamilyMember) -> &[u8] {
    &member.source
}

/// The selected source. It names that source and supplies neither begin
/// authority nor hold release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Member name.
    pub name: String,
    /// Member round.
    pub round: String,
    /// Exact selected source bytes.
    pub source: Vec<u8>,
}

/// One row from [`list_family`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    /// Member name.
    pub name: String,
    /// Member round.
    pub round: String,
    /// Member stance.
    pub stance: Stance,
    /// Deck path.
    pub path: String,
    /// Roadmap id text.
    pub roadmap: String,
    /// Whether the deck has this path and these exact bytes.
    pub in_deck: bool,
    /// Whether the roadmap id equals the spec id.
    pub roadmap_matches: bool,
}

/// One member shown with its grounding against the deck and the spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberView {
    /// Parsed member.
    pub member: FamilyMember,
    /// Whether the deck has this path and these exact bytes.
    pub in_deck: bool,
    /// Whether the roadmap id equals the spec id.
    pub roadmap_matches: bool,
}

/// Structural difference of two members, plus grounding flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberDiff {
    /// Left member name.
    pub left: String,
    /// Right member name.
    pub right: String,
    /// Whether the exact source bytes differ.
    pub bytes_differ: bool,
    /// Section names whose stored text differs, in source order.
    pub changed: Vec<&'static str>,
    /// Whether the left member is in the deck.
    pub left_in_deck: bool,
    /// Whether the right member is in the deck.
    pub right_in_deck: bool,
    /// Whether the left roadmap id equals the spec id.
    pub left_roadmap_matches: bool,
    /// Whether the right roadmap id equals the spec id.
    pub right_roadmap_matches: bool,
}

/// Selected source compiled into the existing values.
#[derive(Clone, Debug)]
pub struct CompiledAuthoring {
    /// Exact selected source bytes.
    pub source: Vec<u8>,
    /// Selection. It has no begin authority and no hold release.
    pub selection: Selection,
    /// Opened revision. It is not committed.
    pub revision: crate::Revision,
    /// Work value filled from the source. It is not received.
    pub work: crate::CanonicalWork,
    /// Source-deck proof leaf for the selected facts.
    pub proof: crate::ProofLeaf,
    /// Sealed mutation plan. It is not applied.
    pub mutation: crate::SealedMutationPlan,
    /// Content ids of consulted members' exact bytes. They are not claim inputs.
    pub consultation: Vec<sim_kernel::ContentId>,
}

/// Why a family source, a lint, a selection, or a compile refused.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AuthorError {
    /// The bytes are not UTF-8.
    #[error("source is not utf-8")]
    Utf8,
    /// The first text line is not `family/v1`.
    #[error("source does not start with family/v1")]
    Header,
    /// A required section is absent or empty.
    #[error("missing {section}")]
    Missing {
        /// Section name.
        section: &'static str,
    },
    /// A single-value section or a fact id was repeated.
    #[error("duplicate {section}")]
    Duplicate {
        /// Section name.
        section: &'static str,
    },
    /// The line key is not a family field.
    #[error("unknown field {name}")]
    UnknownField {
        /// Field key.
        name: String,
    },
    /// The stance token is not one of the four stances.
    #[error("stance refused")]
    Stance,
    /// The path is empty, absolute, too long, reserved, or escapes.
    #[error("path refused")]
    Path,
    /// A name, round, owner, roadmap id, or fact id is not admissible.
    #[error("identity refused: {kind}")]
    Identity {
        /// Which identity refused.
        kind: &'static str,
    },
    /// `from_members` received no members.
    #[error("family is empty")]
    EmptyFamily,
    /// Two members share a name or a round.
    #[error("duplicate member {kind}")]
    DuplicateMember {
        /// `name` or `round`.
        kind: &'static str,
    },
    /// The family does not contain exactly one selected member.
    #[error("selection count is {count}")]
    SelectionCount {
        /// How many members are selected.
        count: usize,
    },
    /// No deck file has this path and these exact bytes.
    #[error("deck bytes differ for {path}")]
    DeckMismatch {
        /// Path that failed the byte check.
        path: String,
    },
    /// A member names a roadmap id other than the spec id.
    #[error("roadmap id {found} differs from {spec}")]
    RoadmapMismatch {
        /// Id text on the member.
        found: String,
        /// Id text on the spec.
        spec: String,
    },
    /// The spec has no leaf checkpoint to ground the proof.
    #[error("roadmap has no requirement")]
    Ungrounded,
    /// The selected member does not itself name this requirement.
    #[error("requirement {requirement} is not satisfied by the selected source")]
    Unsatisfied {
        /// Checkpoint id.
        requirement: String,
    },
    /// A selected decision names begin authority or hold release.
    #[error("selection names begin authority or hold release")]
    Authority,
    /// A consult line does not name a consulted round.
    #[error("consult round {round} is not consultation")]
    Consult {
        /// Round text.
        round: String,
    },
    /// The named member is not in the family.
    #[error("member {name} is absent")]
    MemberMissing {
        /// Member name.
        name: String,
    },
    /// The roadmap spec is not admissible.
    #[error("roadmap refused: {0}")]
    Roadmap(String),
    /// Opening the revision refused.
    #[error("revision refused: {0}")]
    Revision(String),
    /// The proof leaf is ungrounded.
    #[error("proof refused: {0}")]
    Proof(String),
    /// Sealing the mutation plan refused.
    #[error("mutation refused: {0}")]
    Mutation(String),
}

pub(super) fn member_in_deck(deck: &SourceDeck, member: &FamilyMember) -> bool {
    deck.files()
        .iter()
        .any(|file| file.path == member.path && file.bytes == member.source)
}

pub(super) fn member_roadmap_matches(spec: &RoadmapSpec, member: &FamilyMember) -> bool {
    member.roadmap == spec.id.as_str()
}

pub(super) fn ensure_roadmap(spec: &RoadmapSpec) -> Result<(), AuthorError> {
    spec.admit()
        .map(|_| ())
        .map_err(|error| AuthorError::Roadmap(format!("{error:?}")))
}

#[cfg(test)]
#[path = "author_fixture.rs"]
mod fixture;
#[cfg(test)]
#[path = "author_more_tests.rs"]
mod more_tests;
#[cfg(test)]
#[path = "author_tests.rs"]
mod tests;
