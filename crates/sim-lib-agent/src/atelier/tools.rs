//! Typed, capability-checked Atelier tool adapters.

use super::{
    AgentMission, AtelierAction, GuardCapability, GuardDecision, GuardRefusal, guard_action,
};
use sim_kernel::{Expr, Result, Symbol};
use sim_lib_stream_core::{DevCassette, DevEvent, LatencyClass};

/// Static descriptor for one auditable agent tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierToolDescriptor {
    /// Stable descriptor id.
    pub id: String,
    /// Human-facing title.
    pub title: String,
    /// Exact command or command template.
    pub command: String,
    /// Guard capability required before action.
    pub required_capability: GuardCapability,
    /// DevEnvelope event kind recorded on success.
    pub evidence_kind: String,
    /// Optional repository scope.
    pub repo: Option<String>,
}

impl AtelierToolDescriptor {
    fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        command: impl Into<String>,
        required_capability: GuardCapability,
        evidence_kind: impl Into<String>,
        repo: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            command: command.into(),
            required_capability,
            evidence_kind: evidence_kind.into(),
            repo,
        }
    }

    /// Encodes this descriptor as ordinary SIM expression data.
    pub fn as_expr(&self) -> Expr {
        let mut entries = vec![
            key("id", Expr::String(self.id.clone())),
            key("title", Expr::String(self.title.clone())),
            key("command", Expr::String(self.command.clone())),
            key(
                "capability",
                Expr::String(capability_label(&self.required_capability)),
            ),
            key("evidence-kind", Expr::String(self.evidence_kind.clone())),
        ];
        if let Some(repo) = &self.repo {
            entries.push(key("repo", Expr::String(repo.clone())));
        }
        Expr::Map(entries)
    }
}

/// Request to update one `repos.toml` commit pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinUpdateRequest {
    /// Repository whose pin is being updated.
    pub repo: String,
    /// Currently pinned commit.
    pub current_commit: String,
    /// Commit the pin should move to.
    pub new_commit: String,
    /// Whether the new commit already exists on the pushed upstream remote.
    pub pushed_commit_exists: bool,
}

/// Docs operation requested by an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocsOperation {
    /// Regenerate docs via tooling.
    Regenerate,
    /// Hand-edit docs source directly.
    HandEdit,
}

/// Request to run docs tooling or edit docs source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocsRegenerationRequest {
    /// Repository whose docs are affected.
    pub repo: String,
    /// Docs source path being run or edited.
    pub path: String,
    /// Command that regenerates the docs.
    pub docs_command: String,
    /// Whether the path is a generated public doc.
    pub generated_public_doc: bool,
    /// Whether docs are regenerated or hand-edited.
    pub operation: DocsOperation,
}

/// Evidence emitted by a completed validation or docs tool run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolRunEvidence {
    /// Descriptor of the tool that ran.
    pub descriptor: AtelierToolDescriptor,
    /// Process exit status of the run.
    pub exit_status: i32,
    /// Path to the captured run log.
    pub log_path: String,
}

impl ToolRunEvidence {
    /// Builds evidence for one command run.
    pub fn new(
        descriptor: AtelierToolDescriptor,
        exit_status: i32,
        log_path: impl Into<String>,
    ) -> Self {
        Self {
            descriptor,
            exit_status,
            log_path: log_path.into(),
        }
    }
}

/// One typed tool action requested by an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtelierToolAction {
    /// Run a `simctl` control-surface command.
    Simctl(AtelierToolDescriptor),
    /// Record evidence from a validation tool run.
    Validation(ToolRunEvidence),
    /// Record evidence from a docs tool run.
    Docs(ToolRunEvidence),
    /// Plan a `repos.toml` commit pin update.
    PinUpdate(PinUpdateRequest),
    /// Run or edit docs through a regeneration request.
    DocsRegeneration(DocsRegenerationRequest),
}

/// Result of one tool action evaluation and its evidence cassette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierToolEvaluation {
    /// Descriptor of the evaluated tool action.
    pub descriptor: AtelierToolDescriptor,
    /// Guard decision for the action.
    pub decision: GuardDecision,
    /// Evidence cassette recording the decision.
    pub cassette: DevCassette,
}

/// Returns descriptors for the committed `simctl` control surface used by agents.
///
/// The caller supplies the control-plane repository (`control_repo`, whose
/// validation command gates most control actions) and the front-page docs
/// repository (`docs_repo`, whose docs the `site` command regenerates).
pub fn simctl_tool_descriptors(
    control_repo: impl Into<String>,
    docs_repo: impl Into<String>,
) -> Vec<AtelierToolDescriptor> {
    let control_repo = control_repo.into();
    let docs_repo = docs_repo.into();
    [
        ("clone", "Clone or update sibling repos"),
        ("meta-build", "Regenerate the validation meta workspace"),
        ("audit", "Run the private-data audit"),
        ("no-github-check", "Assert no GitHub work is enabled"),
        ("site", "Regenerate the public front page"),
        ("repos", "List the constellation manifest"),
        ("atelier-site", "Emit the Atelier Site graph"),
        ("atelier-index", "Refresh the Atelier index"),
        ("atelier-radar", "Query ranked Atelier hints"),
        ("atelier-guard", "Run the Guideline Firewall"),
    ]
    .into_iter()
    .map(|(command, title)| {
        let capability = if command == "site" {
            GuardCapability::RegenDocs(docs_repo.clone())
        } else {
            GuardCapability::RunValidation(control_repo.clone())
        };
        AtelierToolDescriptor::new(
            format!("simctl/{command}"),
            title,
            format!("sh bin/simctl {command}"),
            capability,
            "control",
            Some(control_repo.clone()),
        )
    })
    .collect()
}

/// Builds a descriptor for a repo `validation_command`.
pub fn repo_validation_descriptor(
    repo: impl Into<String>,
    validation_command: impl Into<String>,
) -> AtelierToolDescriptor {
    let repo = repo.into();
    AtelierToolDescriptor::new(
        format!("validation/{repo}"),
        format!("Validate {repo}"),
        validation_command,
        GuardCapability::RunValidation(repo.clone()),
        "validate",
        Some(repo),
    )
}

/// Builds a descriptor for a repo `docs_command`.
pub fn repo_docs_descriptor(
    repo: impl Into<String>,
    docs_command: impl Into<String>,
) -> AtelierToolDescriptor {
    let repo = repo.into();
    AtelierToolDescriptor::new(
        format!("docs/{repo}"),
        format!("Regenerate docs for {repo}"),
        docs_command,
        GuardCapability::RegenDocs(repo.clone()),
        "docs",
        Some(repo),
    )
}

/// Evaluates a typed tool action, enforcing its guard and recording evidence.
pub fn evaluate_atelier_tool(
    mission: &AgentMission,
    action: AtelierToolAction,
    atelier_node: Symbol,
    stream_id: Symbol,
) -> Result<AtelierToolEvaluation> {
    let descriptor = action.descriptor();
    let guard = action.guard_action();
    let decision = match guard_action(mission, guard.clone()) {
        GuardDecision::Granted => action.post_guard_decision(guard),
        refused @ GuardDecision::Refused(_) => refused,
    };
    let event = match &decision {
        GuardDecision::Granted => DevEvent::new(
            &descriptor.evidence_kind,
            atelier_node,
            LatencyClass::OfflineRender,
            action.payload_expr(&descriptor),
        )?,
        GuardDecision::Refused(refusal) => refusal.dev_event(atelier_node)?,
    };
    Ok(AtelierToolEvaluation {
        descriptor,
        decision,
        cassette: DevCassette::from_events(stream_id, vec![event])?,
    })
}

impl AtelierToolAction {
    fn descriptor(&self) -> AtelierToolDescriptor {
        match self {
            Self::Simctl(descriptor) => descriptor.clone(),
            Self::Validation(evidence) | Self::Docs(evidence) => evidence.descriptor.clone(),
            Self::PinUpdate(request) => AtelierToolDescriptor::new(
                format!("pin/{}", request.repo),
                format!("Update {} pin", request.repo),
                format!(
                    "set repos.toml {} commit {}",
                    request.repo, request.new_commit
                ),
                GuardCapability::PlanPin,
                "pin",
                Some(request.repo.clone()),
            ),
            Self::DocsRegeneration(request) => {
                repo_docs_descriptor(request.repo.clone(), request.docs_command.clone())
            }
        }
    }

    fn guard_action(&self) -> AtelierAction {
        match self {
            Self::Simctl(descriptor) => guard_action_for_descriptor(descriptor),
            Self::Validation(evidence) | Self::Docs(evidence) => {
                guard_action_for_descriptor(&evidence.descriptor)
            }
            Self::PinUpdate(request) => AtelierAction::PlanPin {
                repo: request.repo.clone(),
            },
            Self::DocsRegeneration(request) => AtelierAction::RegenDocs {
                repo: request.repo.clone(),
            },
        }
    }

    fn post_guard_decision(&self, guard: AtelierAction) -> GuardDecision {
        match self {
            Self::PinUpdate(request) if !request.pushed_commit_exists => refused(
                guard,
                "pin update requires an existing pushed upstream commit",
            ),
            Self::DocsRegeneration(request)
                if request.generated_public_doc && request.operation == DocsOperation::HandEdit =>
            {
                refused(
                    guard,
                    "generated public docs must be regenerated, not hand-edited",
                )
            }
            _ => GuardDecision::Granted,
        }
    }

    fn payload_expr(&self, descriptor: &AtelierToolDescriptor) -> Expr {
        let mut entries = vec![
            key("tool", Expr::String(descriptor.id.clone())),
            key("command", Expr::String(descriptor.command.clone())),
            key(
                "capability",
                Expr::String(capability_label(&descriptor.required_capability)),
            ),
        ];
        match self {
            Self::Validation(evidence) | Self::Docs(evidence) => {
                entries.push(key(
                    "exit-status",
                    Expr::String(evidence.exit_status.to_string()),
                ));
                entries.push(key("log-path", Expr::String(evidence.log_path.clone())));
            }
            Self::PinUpdate(request) => {
                entries.push(key(
                    "current-commit",
                    Expr::String(request.current_commit.clone()),
                ));
                entries.push(key("new-commit", Expr::String(request.new_commit.clone())));
            }
            Self::DocsRegeneration(request) => {
                entries.push(key("path", Expr::String(request.path.clone())));
                entries.push(key(
                    "generated-public-doc",
                    Expr::Bool(request.generated_public_doc),
                ));
            }
            Self::Simctl(_) => {}
        }
        Expr::Map(entries)
    }
}

fn guard_action_for_descriptor(descriptor: &AtelierToolDescriptor) -> AtelierAction {
    match &descriptor.required_capability {
        GuardCapability::RegenDocs(repo) => AtelierAction::RegenDocs { repo: repo.clone() },
        GuardCapability::RunValidation(repo) => AtelierAction::RunValidation { repo: repo.clone() },
        GuardCapability::PlanPin => AtelierAction::PlanPin {
            repo: descriptor.repo.clone().unwrap_or_default(),
        },
        GuardCapability::EvalGated => AtelierAction::Eval {
            label: descriptor.id.clone(),
        },
        GuardCapability::EditRepo(repo) => AtelierAction::EditFile {
            repo: repo.clone(),
            path: descriptor.command.clone(),
        },
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

fn key(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}
