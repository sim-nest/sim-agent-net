//! Guard tokens and refusal recording for Atelier actions.

use super::mission::AgentMission;
use sim_kernel::{Expr, Result, Symbol};
use sim_lib_stream_core::{DevCassette, DevEvent};

/// Capability a mission must hold to perform a guarded Atelier action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardCapability {
    /// Permission to edit files in the named repository.
    EditRepo(String),
    /// Permission to regenerate docs for the named repository.
    RegenDocs(String),
    /// Permission to plan a `repos.toml` commit pin update.
    PlanPin,
    /// Permission to run the named repository's validation command.
    RunValidation(String),
    /// Permission to run a gated eval action.
    EvalGated,
}

/// Concrete action an agent requests the guard to evaluate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtelierAction {
    /// Edit a file within a repository.
    EditFile {
        /// Target repository name.
        repo: String,
        /// File path within the repository.
        path: String,
    },
    /// Regenerate docs for a repository.
    RegenDocs {
        /// Target repository name.
        repo: String,
    },
    /// Plan a `repos.toml` commit pin update for a repository.
    PlanPin {
        /// Target repository name.
        repo: String,
    },
    /// Run a repository's validation command.
    RunValidation {
        /// Target repository name.
        repo: String,
    },
    /// Run a gated eval action.
    Eval {
        /// Label identifying the eval.
        label: String,
    },
    /// Add a GitHub remote (always hard-denied).
    AddGithubRemote {
        /// Remote being added.
        remote: String,
    },
    /// Flip a repository's `publish_to_github` flag (always hard-denied).
    FlipPublishToGithub {
        /// Target repository name.
        repo: String,
    },
    /// Push to a mirror remote (always hard-denied).
    PushMirrorRemote,
}

impl AtelierAction {
    /// Builds an [`AtelierAction::EditFile`] from repo and path.
    pub fn edit_file(repo: impl Into<String>, path: impl Into<String>) -> Self {
        Self::EditFile {
            repo: repo.into(),
            path: path.into(),
        }
    }

    /// Returns the repository this action targets, if any.
    pub fn repo(&self) -> Option<&str> {
        match self {
            Self::EditFile { repo, .. }
            | Self::RegenDocs { repo }
            | Self::PlanPin { repo }
            | Self::RunValidation { repo }
            | Self::FlipPublishToGithub { repo } => Some(repo),
            Self::Eval { .. } | Self::AddGithubRemote { .. } | Self::PushMirrorRemote => None,
        }
    }

    /// Returns a short human-readable label for this action.
    pub fn label(&self) -> String {
        match self {
            Self::EditFile { repo, path } => format!("edit {repo}:{path}"),
            Self::RegenDocs { repo } => format!("regen-docs {repo}"),
            Self::PlanPin { repo } => format!("plan-pin {repo}"),
            Self::RunValidation { repo } => format!("run-validation {repo}"),
            Self::Eval { label } => format!("eval {label}"),
            Self::AddGithubRemote { remote } => format!("add-github-remote {remote}"),
            Self::FlipPublishToGithub { repo } => format!("flip-publish-to-github {repo}"),
            Self::PushMirrorRemote => "push-mirror-remote".to_owned(),
        }
    }
}

/// Record of a denied action paired with the reason it was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardRefusal {
    action: AtelierAction,
    reason: String,
}

impl GuardRefusal {
    /// Builds a refusal for an action and its reason.
    pub fn new(action: AtelierAction, reason: impl Into<String>) -> Self {
        Self {
            action,
            reason: reason.into(),
        }
    }

    /// Returns the refused action.
    pub fn action(&self) -> &AtelierAction {
        &self.action
    }

    /// Returns the human-readable refusal reason.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Encodes this refusal as a refusal `DevEvent` for the given node.
    pub fn dev_event(&self, atelier_node: Symbol) -> Result<DevEvent> {
        DevEvent::refusal(atelier_node, self.payload_expr())
    }

    fn payload_expr(&self) -> Expr {
        Expr::Map(vec![
            (
                Expr::Symbol(Symbol::new("refusal")),
                Expr::Symbol(Symbol::qualified("atelier/refusal", "guard")),
            ),
            (
                Expr::Symbol(Symbol::new("action")),
                Expr::String(self.action.label()),
            ),
            (
                Expr::Symbol(Symbol::new("reason")),
                Expr::String(self.reason.clone()),
            ),
        ])
    }
}

/// Outcome of guarding an action: granted or refused with a reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardDecision {
    /// The action is permitted.
    Granted,
    /// The action is denied, carrying the refusal record.
    Refused(GuardRefusal),
}

impl GuardDecision {
    /// Returns true when the action was granted.
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }

    /// Returns the refusal record when the action was refused.
    pub fn refusal(&self) -> Option<&GuardRefusal> {
        match self {
            Self::Granted => None,
            Self::Refused(refusal) => Some(refusal),
        }
    }
}

/// Guard decision plus any evidence cassette recorded for a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardEvaluation {
    decision: GuardDecision,
    cassette: Option<DevCassette>,
}

impl GuardEvaluation {
    /// Returns the guard decision.
    pub fn decision(&self) -> &GuardDecision {
        &self.decision
    }

    /// Returns the evidence cassette recorded for a refusal, if any.
    pub fn cassette(&self) -> Option<&DevCassette> {
        self.cassette.as_ref()
    }
}

/// Guards an action and records a refusal cassette when it is denied.
pub fn evaluate_guarded_action(
    mission: &AgentMission,
    action: AtelierAction,
    atelier_node: Symbol,
    stream_id: Symbol,
) -> Result<GuardEvaluation> {
    let decision = guard_action(mission, action);
    let cassette = match &decision {
        GuardDecision::Granted => None,
        GuardDecision::Refused(refusal) => Some(DevCassette::from_events(
            stream_id,
            vec![refusal.dev_event(atelier_node)?],
        )?),
    };
    Ok(GuardEvaluation { decision, cassette })
}

/// Decides an action against hard denials and the mission's capabilities.
pub fn guard_action(mission: &AgentMission, action: AtelierAction) -> GuardDecision {
    if let Some(reason) = hard_denial_reason(mission, &action) {
        return refused(action, reason);
    }
    let Some(required) = required_capability(&action) else {
        return GuardDecision::Granted;
    };
    if mission.capabilities().contains(&required) {
        GuardDecision::Granted
    } else {
        refused(
            action,
            format!("missing guard capability {}", capability_label(&required)),
        )
    }
}

fn hard_denial_reason(mission: &AgentMission, action: &AtelierAction) -> Option<String> {
    match action {
        AtelierAction::EditFile { repo, path } => {
            if path_has_meta_workspace(path) {
                return Some("edits under .meta-workspace are denied".to_owned());
            }
            if repo != mission.leased_repo() {
                return Some(format!(
                    "mission lease is {}, not {repo}",
                    mission.leased_repo()
                ));
            }
            if mission.code_free_repos().iter().any(|r| r == repo) && path.ends_with(".rs") {
                return Some(format!("Rust source is denied in {repo}"));
            }
            None
        }
        AtelierAction::AddGithubRemote { .. } => {
            Some("adding a GitHub remote is denied".to_owned())
        }
        AtelierAction::FlipPublishToGithub { .. } => {
            Some("changing publish_to_github is denied".to_owned())
        }
        AtelierAction::PushMirrorRemote => Some("pushing to a mirror remote is denied".to_owned()),
        AtelierAction::RegenDocs { .. }
        | AtelierAction::PlanPin { .. }
        | AtelierAction::RunValidation { .. }
        | AtelierAction::Eval { .. } => None,
    }
}

fn required_capability(action: &AtelierAction) -> Option<GuardCapability> {
    match action {
        AtelierAction::EditFile { repo, .. } => Some(GuardCapability::EditRepo(repo.clone())),
        AtelierAction::RegenDocs { repo } => Some(GuardCapability::RegenDocs(repo.clone())),
        AtelierAction::PlanPin { .. } => Some(GuardCapability::PlanPin),
        AtelierAction::RunValidation { repo } => Some(GuardCapability::RunValidation(repo.clone())),
        AtelierAction::Eval { .. } => Some(GuardCapability::EvalGated),
        AtelierAction::AddGithubRemote { .. }
        | AtelierAction::FlipPublishToGithub { .. }
        | AtelierAction::PushMirrorRemote => None,
    }
}

fn refused(action: AtelierAction, reason: impl Into<String>) -> GuardDecision {
    GuardDecision::Refused(GuardRefusal::new(action, reason))
}

fn capability_label(capability: &GuardCapability) -> String {
    match capability {
        GuardCapability::EditRepo(repo) => format!("EditRepo({repo})"),
        GuardCapability::RegenDocs(repo) => format!("RegenDocs({repo})"),
        GuardCapability::PlanPin => "PlanPin".to_owned(),
        GuardCapability::RunValidation(repo) => format!("RunValidation({repo})"),
        GuardCapability::EvalGated => "EvalGated".to_owned(),
    }
}

fn path_has_meta_workspace(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|component| component == ".meta-workspace")
}
