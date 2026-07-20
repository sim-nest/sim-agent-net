use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use sim_codec_bridge::{
    BridgeFrameBook, BridgeFramePayload, BridgePart, FrameHoleKind, FrameHoleSpec, FrameKind,
    FrameSpec, frame_book_content_id,
};
use sim_kernel::{ContentId, Cx, Datum, Error, Expr, Result, Symbol};
use sim_value::build::entry;

use crate::semantic_tokens;

/// Candidate frame specification inferred from prose that has no registered frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameSpecProposal {
    /// Proposed frame id.
    pub id: Symbol,
    /// Proposed illocutionary frame kind.
    pub kind: FrameKind,
    /// Proposed deterministic template body using `{hole}` placeholders.
    pub template: String,
    /// Proposed typed holes accepted by the template.
    pub holes: Vec<FrameHoleSpec>,
    /// Short explanation of why this frame fits the prose fragment.
    pub rationale: String,
}

impl FrameSpecProposal {
    /// Returns the proposed hole names as stable strings.
    pub fn hole_names(&self) -> Vec<String> {
        self.holes
            .iter()
            .map(|hole| hole.name.as_qualified_str().to_owned())
            .collect()
    }

    /// Encodes the proposal as data for model repair prompts and review views.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            entry("id", Expr::Symbol(self.id.clone())),
            entry("kind", Expr::Symbol(frame_kind_symbol(self.kind))),
            entry("template", Expr::String(self.template.clone())),
            entry(
                "holes",
                Expr::Vector(
                    self.holes
                        .iter()
                        .map(|hole| {
                            Expr::Map(vec![
                                entry("name", Expr::Symbol(hole.name.clone())),
                                entry("kind", Expr::Symbol(hole_kind_symbol(hole.kind))),
                            ])
                        })
                        .collect(),
                ),
            ),
            entry("rationale", Expr::String(self.rationale.clone())),
            entry(
                "status",
                Expr::Symbol(Symbol::qualified("forge", "candidate")),
            ),
        ])
    }

    /// Computes the content id of this candidate proposal.
    pub fn content_id(&self) -> Result<ContentId> {
        proposal_datum(self).content_id()
    }

    fn frame_spec(&self) -> FrameSpec {
        FrameSpec::new(
            self.id.clone(),
            self.kind,
            intern_template(&self.template),
            self.holes.clone(),
        )
    }
}

/// Proposes a typed frame spec for prose that has no registered frame.
pub fn propose_frame(_cx: &mut Cx, prose_fragment: &str) -> Result<FrameSpecProposal> {
    let prose = prose_fragment.trim();
    if prose.is_empty() {
        return Err(Error::Eval(
            "forge frame proposal requires non-empty prose".to_owned(),
        ));
    }
    let tokens = semantic_tokens(prose);
    if tokens.is_empty() {
        return Err(Error::Eval(
            "forge frame proposal requires at least one token".to_owned(),
        ));
    }
    let kind = infer_kind(prose);
    let (template, holes) = proposal_template(kind, &tokens);
    let id = Symbol::qualified("forge", frame_slug(kind, &tokens).as_str());
    Ok(FrameSpecProposal {
        id,
        kind,
        template,
        holes,
        rationale: format!(
            "The prose is mapped to a typed {} frame instead of prose data.",
            frame_kind_label(kind)
        ),
    })
}

/// Registers a human-approved proposal in `book` and returns the new book id.
pub fn approve_frame_proposal(
    book: &mut BridgeFrameBook,
    proposal: &FrameSpecProposal,
) -> Result<ContentId> {
    book.register(proposal.frame_spec());
    frame_book_content_id(book)
}

/// Builds a normative frame part from an approved proposal and matching prose.
pub fn proposed_frame_part(
    book: &BridgeFrameBook,
    proposal: &FrameSpecProposal,
    prose_fragment: &str,
    id: Symbol,
) -> Result<BridgePart> {
    book.require_spec(&proposal.id)?;
    let payload = proposal_payload(proposal, prose_fragment)?;
    book.validate_payload(&payload.to_expr())?;
    Ok(BridgePart {
        id,
        kind: Symbol::qualified("bridge", "Frame"),
        payload: payload.to_expr(),
    })
}

fn proposal_payload(
    proposal: &FrameSpecProposal,
    prose_fragment: &str,
) -> Result<BridgeFramePayload> {
    let tokens = semantic_tokens(prose_fragment);
    if tokens.is_empty() {
        return Err(Error::Eval(
            "forge frame proposal cannot fill slots from empty prose".to_owned(),
        ));
    }
    let mut payload = BridgeFramePayload::new(proposal.id.clone());
    for hole in &proposal.holes {
        payload = payload.with_slot(hole.name.clone(), slot_value(hole, &tokens, prose_fragment));
    }
    Ok(payload)
}

fn proposal_template(kind: FrameKind, tokens: &[String]) -> (String, Vec<FrameHoleSpec>) {
    match kind {
        FrameKind::Use => (
            "{resource}.".to_owned(),
            vec![FrameHoleSpec::new(
                Symbol::new("resource"),
                FrameHoleKind::Ref,
            )],
        ),
        FrameKind::Inform => (
            "{fact}.".to_owned(),
            vec![FrameHoleSpec::new(Symbol::new("fact"), FrameHoleKind::Term)],
        ),
        FrameKind::Require | FrameKind::Forbid => (
            "{rule}.".to_owned(),
            vec![FrameHoleSpec::new(Symbol::new("rule"), FrameHoleKind::Term)],
        ),
        FrameKind::Prefer => (
            "{choice}.".to_owned(),
            vec![FrameHoleSpec::new(
                Symbol::new("choice"),
                FrameHoleKind::Choice,
            )],
        ),
        FrameKind::Return => (
            "{shape}.".to_owned(),
            vec![FrameHoleSpec::new(
                Symbol::new("shape"),
                FrameHoleKind::Term,
            )],
        ),
        FrameKind::Check => (
            "{path}.".to_owned(),
            vec![FrameHoleSpec::new(Symbol::new("path"), FrameHoleKind::Path)],
        ),
        FrameKind::Task => {
            let target = tokens.get(1).or_else(|| tokens.first());
            let holes = if target.is_some() {
                vec![
                    FrameHoleSpec::new(Symbol::new("action"), FrameHoleKind::Term),
                    FrameHoleSpec::new(Symbol::new("target"), FrameHoleKind::Term),
                ]
            } else {
                vec![FrameHoleSpec::new(
                    Symbol::new("action"),
                    FrameHoleKind::Term,
                )]
            };
            let template = if holes.len() == 2 {
                "{action} {target}.".to_owned()
            } else {
                "{action}.".to_owned()
            };
            (template, holes)
        }
    }
}

fn slot_value(hole: &FrameHoleSpec, tokens: &[String], prose_fragment: &str) -> Expr {
    match hole.kind {
        FrameHoleKind::Path => Expr::Vector(
            tokens
                .iter()
                .map(|token| Expr::String(token.clone()))
                .collect(),
        ),
        FrameHoleKind::Prose => Expr::String(prose_fragment.trim().to_owned()),
        FrameHoleKind::Ref
        | FrameHoleKind::Term
        | FrameHoleKind::Choice
        | FrameHoleKind::Number => {
            let token = match hole.name.name.as_ref() {
                "target" => tokens.last(),
                "resource" | "fact" | "rule" | "choice" | "shape" => tokens.last(),
                _ => tokens.first(),
            }
            .expect("proposal slots require at least one token");
            Expr::Symbol(Symbol::new(token.as_str()))
        }
    }
}

fn infer_kind(prose: &str) -> FrameKind {
    let lower = prose.trim().to_ascii_lowercase();
    if lower.starts_with("use ") {
        FrameKind::Use
    } else if lower.starts_with("return ") {
        FrameKind::Return
    } else if lower.starts_with("check ") || lower.starts_with("verify ") {
        FrameKind::Check
    } else if lower.starts_with("prefer ") {
        FrameKind::Prefer
    } else if lower.contains("must not") || lower.contains("never ") || lower.starts_with("do not ")
    {
        FrameKind::Forbid
    } else if lower.starts_with("require ") || lower.contains(" must ") {
        FrameKind::Require
    } else if lower.starts_with("note ") || lower.starts_with("inform ") {
        FrameKind::Inform
    } else {
        FrameKind::Task
    }
}

fn frame_slug(kind: FrameKind, tokens: &[String]) -> String {
    let mut parts = vec!["frame".to_owned(), frame_kind_label(kind).to_owned()];
    parts.extend(tokens.iter().take(3).cloned());
    parts.join("-")
}

fn proposal_datum(proposal: &FrameSpecProposal) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("forge", "FrameSpecProposal"),
        fields: vec![
            (Symbol::new("id"), Datum::Symbol(proposal.id.clone())),
            (
                Symbol::new("kind"),
                Datum::Symbol(frame_kind_symbol(proposal.kind)),
            ),
            (
                Symbol::new("template"),
                Datum::String(proposal.template.clone()),
            ),
            (
                Symbol::new("holes"),
                Datum::Vector(
                    proposal
                        .holes
                        .iter()
                        .map(|hole| Datum::Node {
                            tag: Symbol::qualified("forge", "FrameHoleProposal"),
                            fields: vec![
                                (Symbol::new("name"), Datum::Symbol(hole.name.clone())),
                                (
                                    Symbol::new("kind"),
                                    Datum::Symbol(hole_kind_symbol(hole.kind)),
                                ),
                            ],
                        })
                        .collect(),
                ),
            ),
            (
                Symbol::new("rationale"),
                Datum::String(proposal.rationale.clone()),
            ),
        ],
    }
}

fn frame_kind_label(kind: FrameKind) -> &'static str {
    match kind {
        FrameKind::Use => "use",
        FrameKind::Inform => "inform",
        FrameKind::Task => "task",
        FrameKind::Require => "require",
        FrameKind::Forbid => "forbid",
        FrameKind::Prefer => "prefer",
        FrameKind::Return => "return",
        FrameKind::Check => "check",
    }
}

fn frame_kind_symbol(kind: FrameKind) -> Symbol {
    let name = match kind {
        FrameKind::Use => "Use",
        FrameKind::Inform => "Inform",
        FrameKind::Task => "Task",
        FrameKind::Require => "Require",
        FrameKind::Forbid => "Forbid",
        FrameKind::Prefer => "Prefer",
        FrameKind::Return => "Return",
        FrameKind::Check => "Check",
    };
    Symbol::qualified("bridge", name)
}

fn hole_kind_symbol(kind: FrameHoleKind) -> Symbol {
    let name = match kind {
        FrameHoleKind::Ref => "Ref",
        FrameHoleKind::Term => "Term",
        FrameHoleKind::Choice => "Choice",
        FrameHoleKind::Path => "Path",
        FrameHoleKind::Number => "Number",
        FrameHoleKind::Prose => "Prose",
    };
    Symbol::qualified("bridge", name)
}

fn intern_template(template: &str) -> &'static str {
    static TEMPLATES: OnceLock<Mutex<BTreeMap<String, &'static str>>> = OnceLock::new();
    let templates = TEMPLATES.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut templates = templates.lock().expect("template interner mutex poisoned");
    if let Some(existing) = templates.get(template) {
        return existing;
    }
    let leaked = Box::leak(template.to_owned().into_boxed_str());
    templates.insert(template.to_owned(), leaked);
    leaked
}
